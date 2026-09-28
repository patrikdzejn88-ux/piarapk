//! Парсер: сбор участников чата ПО АВТОРАМ последних сообщений.
//! Ускорение (как в gramjs-боте): raw messages.getHistory — юзернеймы
//! приходят В ТОМ ЖЕ ответе (вектор users), второй проход не нужен вообще.
//! K параллельных обходчиков по offset-окнам + поддержка остановки.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use grammers_client::client::Client;
use grammers_client::peer::Peer;
use grammers_session::types::PeerRef;

use super::state::AppState;
use super::store::{sanitize_name, UserDatabase};

/// Параллельных обходчиков сообщений.
const MSG_WORKERS: usize = 4;

/// Сообщений за один getHistory (потолок протокола).
const PAGE: i32 = 100;

/// Результат обходчика.
struct WorkerResult {
    /// user_id → username (только авторы, встреченные в сообщениях)
    usernames: HashMap<i64, String>,
    /// id сообщений (для счётчика и дедупа)
    message_ids: HashSet<i32>,
    /// id авторов без username / не из вектора users
    no_username: HashSet<i64>,
    stopped: bool,
}

/// Извлечь (messages, users) из ответа getHistory.
fn extract_payload(
    resp: grammers_tl_types::enums::messages::Messages,
) -> (Vec<grammers_tl_types::enums::Message>, Vec<grammers_tl_types::enums::User>) {
    use grammers_tl_types::enums::messages::Messages as M;
    match resp {
        M::Messages(m) => (m.messages, m.users),
        M::Slice(m) => (m.messages, m.users),
        M::ChannelMessages(m) => (m.messages, m.users),
        M::NotModified(_) => (Vec::new(), Vec::new()),
    }
}

/// Обходчик: страницы getHistory от offset_id (эксклюзивно) вниз,
/// до limit сообщений.
async fn scan_worker(
    client: Client,
    input_peer: grammers_tl_types::enums::InputPeer,
    start_offset_id: i32,
    limit: usize,
    state: &AppState,
    done: &AtomicUsize,
) -> WorkerResult {
    let mut usernames: HashMap<i64, String> = HashMap::new();
    let mut message_ids = HashSet::new();
    let mut no_username: HashSet<i64> = HashSet::new();
    let mut stopped = false;
    let mut got = 0usize;
    let mut offset_id = start_offset_id;

    while got < limit {
        if state.parser_cancelled() {
            stopped = true;
            break;
        }
        let remaining = (limit - got) as i32;
        let request = grammers_tl_types::functions::messages::GetHistory {
            peer: input_peer.clone(),
            offset_id,
            offset_date: 0,
            add_offset: 0,
            limit: PAGE.min(remaining),
            max_id: 0,
            min_id: 0,
            hash: 0,
        };
        let resp = match client.invoke(&request).await {
            Ok(r) => r,
            Err(e) => {
                // сеть/флуд на этом воркере: сохраняем собранное
                log::warn!("parse: getHistory прерван: {e}");
                break;
            }
        };
        let (messages, users) = extract_payload(resp);
        if messages.is_empty() {
            break;
        }

        // юзернеймы из этого же ответа
        let mut page_users: HashMap<i64, String> = HashMap::new();
        for u in &users {
            if let grammers_tl_types::enums::User::User(user) = u {
                if let Some(un) = &user.username {
                    if !un.is_empty() {
                        page_users.insert(user.id, un.clone());
                    }
                }
            }
        }

        let mut min_id = i32::MAX;
        for msg in &messages {
            let (id, from_user) = match msg {
                grammers_tl_types::enums::Message::Message(m) => (
                    m.id,
                    m.from_id.as_ref(),
                ),
                // служебные (вступления и т.п.) — учитываем id для пейджинга,
                // авторы оттуда не нужны
                grammers_tl_types::enums::Message::Service(s) => (s.id, None),
                _ => continue,
            };
            message_ids.insert(id);
            min_id = min_id.min(id);
            if let Some(grammers_tl_types::enums::Peer::User(pu)) = from_user {
                let uid = pu.user_id;
                if let Some(un) = page_users.get(&uid) {
                    usernames.insert(uid, un.clone());
                } else {
                    no_username.insert(uid);
                }
            }
        }
        let page_len = messages.len();
        got += page_len;
        let total_now = done.fetch_add(page_len, Ordering::Relaxed) + page_len;
        state.progress(
            "parse_start",
            serde_json::json!({
                "phase": "messages",
                "done": total_now,
            }),
        );
        if min_id == i32::MAX || min_id <= 1 {
            break; // дошли до начала истории
        }
        offset_id = min_id; // offset_id эксклюзивный: строго старее
    }

    WorkerResult { usernames, message_ids, no_username, stopped }
}

/// Точка входа: parse_start { chat | dialog_id+access_hash, title, limit }.
pub async fn parse_start(
    state: &AppState,
    chat: &str,
    dialog_id: i64,
    access_hash: i64,
    title: &str,
    limit: usize,
) -> Result<serde_json::Value, serde_json::Value> {
    // парсер работает ТОЛЬКО на пуле «parser» (строгая изоляция, без fallback)
    let Some(client) = super::connect::connected_client_strict(state, "parser") else {
        return Err(super::auth::err_json(
            "NO_PARSER_ACCOUNTS",
            "нет подключённых аккаунтов в пуле «Парсер» — добавьте и подключите аккаунт в разделе «Аккаунты» (вкладка «Парсер»)",
        ));
    };
    state.reset_parser_cancel();

    // источник: выбранный диалог аккаунта (dialog_id) ИЛИ ссылка/@username
    let (peer_ref, base_name) = if dialog_id != 0 {
        let pid = grammers_session::types::PeerId::from_bot_api_dialog_id(dialog_id)
            .ok_or_else(|| super::auth::err_json("ERROR", "некорректный dialog_id"))?;
        let name = if title.trim().is_empty() {
            format!("chat{dialog_id}")
        } else {
            sanitize_name(title.trim())
        };
        (
            PeerRef {
                id: pid,
                auth: grammers_session::types::PeerAuth::from_hash(access_hash),
            },
            name,
        )
    } else {
        let peer: Peer = client
            .resolve_username(&super::chats::normalize_chat_link(chat))
            .await
            .map_err(|e| super::auth::err_json("ERROR", format!("ошибка поиска чата: {e}")))?
            .ok_or_else(|| {
                super::auth::err_json("CHAT_NOT_FOUND", "чат не найден (resolve_username=None)")
            })?;
        let peer_ref = peer
            .to_ref()
            .await
            .map_err(|e| super::auth::err_json("ERROR", format!("to_ref: {e}")))?
            .ok_or_else(|| super::auth::err_json("ERROR", "пустой PeerRef"))?;
        let name = sanitize_name(&super::chats::normalize_chat_link(chat));
        (peer_ref, name)
    };

    let limit = limit.clamp(1, 1_000_000);
    let input_peer: grammers_tl_types::enums::InputPeer = (&peer_ref).into();

    // верхняя граница id — для разбиения на окна
    let top_id: i32 = {
        let mut it = client.iter_messages(peer_ref).limit(1);
        match it.next().await {
            Ok(Some(m)) => m.id(),
            _ => 0,
        }
    };

    let done = AtomicUsize::new(0);
    let mut stopped = false;
    let mut usernames: HashMap<i64, String> = HashMap::new();
    let mut no_username: HashSet<i64> = HashSet::new();
    let mut message_ids: HashSet<i32> = HashSet::new();

    if top_id <= 0 {
        log::info!("parse {base_name}: сообщений не найдено");
    } else {
        let per = (limit / MSG_WORKERS).max(1);
        let mut futs = Vec::with_capacity(MSG_WORKERS);
        for k in 0..MSG_WORKERS {
            // k=0: top_id+1 — offset_id эксклюзивный, иначе пропустим самое
            // свежее сообщение
            let span = (k * per) as i32;
            let offset = if k == 0 {
                top_id.saturating_add(1)
            } else {
                top_id.saturating_sub(span)
            };
            if offset <= 0 && k > 0 {
                break;
            }
            futs.push(scan_worker(
                client.clone(),
                input_peer.clone(),
                offset,
                per,
                state,
                &done,
            ));
        }
        let results = futures_util::future::join_all(futs).await;
        for r in results {
            if r.stopped {
                stopped = true;
            }
            usernames.extend(r.usernames);
            no_username.extend(r.no_username);
            message_ids.extend(r.message_ids);
        }
        log::info!(
            "parse {base_name}: сообщений {}, авторов с username {}, без {}",
            message_ids.len(),
            usernames.len(),
            no_username.len()
        );
    }

    // ---------- сохранить базу ----------
    let messages_done = message_ids.len();
    let list: Vec<String> = usernames.into_values().collect();
    let skipped = no_username.len();
    let path = UserDatabase::path_for(&state.dbs_dir(), &base_name);
    let saved = UserDatabase::append_unique(&path, &list)
        .map_err(|e| super::auth::err_json("ERROR", e.to_string()))?;
    log::info!("parse {base_name}: сохранено {saved}");

    Ok(serde_json::json!({
        "saved": saved,
        "skipped": skipped,
        "base": base_name,
        "messages": messages_done,
        "authors": saved + skipped,
        "stopped": stopped,
    }))
}
