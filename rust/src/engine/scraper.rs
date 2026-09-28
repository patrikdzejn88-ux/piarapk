//! Парсер: сбор участников чата ПО АВТОРАМ последних сообщений.
//! Ускорение: (1) K параллельных обходчиков сообщений (по offset_id-окнам);
//! (2) батч-резолв usernames через users.getUsers по 100 штук за запрос,
//! чанки тоже параллельно; (3) поддержка остановки (parse_cancel) с
//! сохранением уже собранного.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use grammers_client::client::Client;
use grammers_client::peer::Peer;
use grammers_session::types::PeerRef;

use super::state::AppState;
use super::store::{sanitize_name, UserDatabase};

/// Параллельных обходчиков сообщений.
const MSG_WORKERS: usize = 4;

/// Сколько InputUser резолвится за один users.getUsers.
const RESOLVE_CHUNK: usize = 100;

/// Параллельно чанков GetUsers.
const RESOLVE_WORKERS: usize = 4;

/// Результат одного обходчика.
struct WorkerResult {
    authors: HashMap<grammers_session::types::PeerId, PeerRef>,
    message_ids: HashSet<i32>,
    stopped: bool,
}

/// Обходчик: сообщения старше offset_id, максимум limit штук.
#[allow(clippy::too_many_arguments)]
async fn scan_worker(
    client: Client,
    peer_ref: PeerRef,
    offset_id: i32,
    limit: usize,
    state: &AppState,
    done: &AtomicUsize,
) -> WorkerResult {
    let mut authors = HashMap::new();
    let mut message_ids = HashSet::new();
    let mut n = 0usize;
    let mut stopped = false;
    let mut iter = if offset_id > 0 {
        client.iter_messages(peer_ref).offset_id(offset_id).limit(limit)
    } else {
        client.iter_messages(peer_ref).limit(limit)
    };
    loop {
        // сеть/флуд на этом воркере: сохраняем что есть
        let next = match iter.next().await {
            Ok(v) => v,
            Err(_) => break,
        };
        let Some(m) = next else { break };
        {
            n += 1;
            message_ids.insert(m.id());
            if let Some(r) = m.sender_ref().await.ok().flatten() {
                authors.insert(r.id, r);
            }
            let total_now = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 100 == 0 {
                state.progress(
                    "parse_start",
                    serde_json::json!({
                        "phase": "messages",
                        "done": total_now,
                        "authors": 0, // обновит агрегатор ниже
                    }),
                );
            }
            if state.parser_cancelled() {
                stopped = true;
                break;
            }
        }
    }
    WorkerResult { authors, message_ids, stopped }
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
            grammers_session::types::PeerRef {
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

    // ---------- фаза 1: K параллельных обходчиков ----------
    let limit = limit.clamp(1, 1_000_000);

    // верхняя граница id (для разбиения на окна)
    let top_id: i32 = {
        let mut it = client.iter_messages(peer_ref).limit(1);
        match it.next().await {
            Ok(Some(m)) => m.id(),
            _ => 0,
        }
    };

    let done = AtomicUsize::new(0);
    let mut stopped = false;
    let mut authors: HashMap<grammers_session::types::PeerId, PeerRef> = HashMap::new();
    let mut message_ids: HashSet<i32> = HashSet::new();

    if top_id <= 0 {
        // пустой чат/нет сообщений — сразу финальная фаза
        log::info!("parse {base_name}: сообщений не найдено");
    } else {
        let per = (limit / MSG_WORKERS).max(1);
        // окна по offset_id: воркер k стартует старее (top_id - k*per)
        let mut futs = Vec::with_capacity(MSG_WORKERS);
        for k in 0..MSG_WORKERS {
            let offset = top_id.saturating_sub((k * per) as i32);
            if offset <= 0 && k > 0 {
                break;
            }
            futs.push(scan_worker(
                client.clone(),
                peer_ref,
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
            authors.extend(r.authors);
            message_ids.extend(r.message_ids);
        }
        log::info!(
            "parse {base_name}: сообщений {}, уникальных авторов {}",
            message_ids.len(),
            authors.len()
        );
    }
    let messages_done = message_ids.len();

    // ---------- фаза 2: параллельный батч-резолв usernames ----------
    let refs: Vec<PeerRef> = authors.values().cloned().collect();
    let total_authors = refs.len();
    let mut usernames: Vec<String> = Vec::new();
    let mut no_username = 0usize;

    let chunks: Vec<&[PeerRef]> = refs.chunks(RESOLVE_CHUNK).collect();
    // параллельно — но ограниченно (RESOLVE_WORKERS за раз): 100+ одновременных
    // GetUsers почти гарантированно ловят FLOOD_WAIT
    let mut resolved = 0usize;
    for group in chunks.chunks(RESOLVE_WORKERS) {
        let group_results = futures_util::future::join_all(
            group.iter().map(|chunk| {
                let client = client.clone();
                async move {
                    let ids: Vec<grammers_tl_types::enums::InputUser> =
                        chunk.iter().map(|r| r.into()).collect();
                    let request =
                        grammers_tl_types::functions::users::GetUsers { id: ids };
                    client.invoke(&request).await
                }
            }),
        )
        .await;

        for res in group_results {
            match res {
                Ok(users) => {
                    for u in users {
                        if let grammers_tl_types::enums::User::User(user) = u {
                            match user.username {
                                Some(un) if !un.is_empty() => usernames.push(un),
                                _ => no_username += 1,
                            }
                        } else {
                            no_username += 1;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("parse: чанк не разрезолвился: {e}");
                    no_username += RESOLVE_CHUNK;
                }
            }
            resolved = (resolved + RESOLVE_CHUNK).min(total_authors);
        }
        state.progress(
            "parse_start",
            serde_json::json!({
                "phase": "resolve",
                "done": resolved,
                "total": total_authors,
            }),
        );
        if state.parser_cancelled() {
            stopped = true;
            break;
        }
    }

    // ---------- фаза 3: сохранить базу ----------
    let path = UserDatabase::path_for(&state.dbs_dir(), &base_name);
    let saved = UserDatabase::append_unique(&path, &usernames)
        .map_err(|e| super::auth::err_json("ERROR", e.to_string()))?;
    log::info!("parse {base_name}: usernames {}, сохранено {saved}", usernames.len());

    Ok(serde_json::json!({
        "saved": saved,
        "skipped": no_username,
        "base": base_name,
        "messages": messages_done,
        "authors": total_authors,
        "stopped": stopped,
    }))
}
