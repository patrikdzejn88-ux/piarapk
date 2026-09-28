//! Парсер: сбор участников чата ПО АВТОРАМ последних сообщений.
//! Оптимизация: (1) обход сообщений без резолва; (2) батч-резолв usernames
//! через users.getUsers по 100 штук за запрос (вместо поштучного resolve_peer —
//! ускорение в 5-10 раз); (3) поддержка остановки (parse_cancel) с
//! сохранением уже собранного.

use std::collections::HashMap;

use grammers_client::client::Client;
use grammers_client::peer::Peer;
use grammers_session::types::PeerRef;

use super::state::AppState;
use super::store::{sanitize_name, UserDatabase};

/// Сколько InputUser резолвится за один users.getUsers.
const RESOLVE_CHUNK: usize = 100;

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

    // ---------- фаза 1: обходим сообщения, собираем УНИКАЛЬНЫХ авторов ----------
    let limit = limit.clamp(1, 1_000_000);
    let mut iter = client.iter_messages(peer_ref).limit(limit);
    // PeerId → PeerRef (dedup)
    let mut authors: HashMap<grammers_session::types::PeerId, PeerRef> = HashMap::new();
    let mut messages_done = 0usize;
    let mut stopped = false;

    while let Some(m) = iter
        .next()
        .await
        .map_err(|e| super::auth::err_json("ERROR", format!("iter_messages: {e}")))?
    {
        messages_done += 1;
        if let Some(r) = m.sender_ref().await.ok().flatten() {
            authors.insert(r.id, r);
        }
        if messages_done % 100 == 0 {
            state.progress(
                "parse_start",
                serde_json::json!({
                    "phase": "messages",
                    "done": messages_done,
                    "total": limit,
                    "authors": authors.len(),
                }),
            );
        }
        if state.parser_cancelled() {
            stopped = true;
            log::info!("parse: остановка пользователем после {messages_done} сообщений");
            break;
        }
    }
    log::info!("parse {base_name}: сообщений {messages_done}, уникальных авторов {}", authors.len());

    // ---------- фаза 2: батч-резолв usernames (users.getUsers по 100) ----------
    let refs: Vec<PeerRef> = authors.values().cloned().collect();
    let total_authors = refs.len();
    let mut usernames: Vec<String> = Vec::new();
    let mut no_username = 0usize;
    let mut failed_chunks = 0usize;

    for (i, chunk) in refs.chunks(RESOLVE_CHUNK).enumerate() {
        if state.parser_cancelled() {
            stopped = true;
            break;
        }
        let ids: Vec<grammers_tl_types::enums::InputUser> =
            chunk.iter().map(|r| r.into()).collect();
        let request = grammers_tl_types::functions::users::GetUsers { id: ids };
        match client.invoke(&request).await {
            Ok(users) => {
                for u in users {
                    if let grammers_tl_types::enums::User::User(user) = u {
                        if let Some(un) = user.username {
                            if !un.is_empty() {
                                usernames.push(un);
                            } else {
                                no_username += 1;
                            }
                        } else {
                            no_username += 1;
                        }
                    } else {
                        no_username += 1;
                    }
                }
            }
            Err(e) => {
                failed_chunks += 1;
                log::warn!("parse: чанк {i} не разрезолвился: {e}");
                no_username += chunk.len();
            }
        }
        let done = (i + 1) * RESOLVE_CHUNK;
        state.progress(
            "parse_start",
            serde_json::json!({
                "phase": "resolve",
                "done": done.min(total_authors),
                "total": total_authors,
                "authors": total_authors,
            }),
        );
    }

    // ---------- фаза 3: сохранить базу ----------
    let path = UserDatabase::path_for(&state.dbs_dir(), &base_name);
    let saved = UserDatabase::append_unique(&path, &usernames)
        .map_err(|e| super::auth::err_json("ERROR", e))?;
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
