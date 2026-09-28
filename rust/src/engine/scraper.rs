//! Парсер: сбор участников чата ПО АВТОРАМ последних сообщений (единственный
//! режим). Порт scrape-логики tg-piar (по сообщениям, дедуп, прогресс).

use std::collections::HashMap;

use grammers_client::client::Client;
use grammers_client::peer::Peer;
use grammers_session::types::PeerRef;

use super::state::AppState;
use super::store::{sanitize_name, UserDatabase};

/// Собрать usernames авторов последних `limit` сообщений чата.
/// Возвращает (сохранено, пропущено).
async fn scrape_message_authors(
    state: &AppState,
    client: &Client,
    peer_ref: PeerRef,
    limit: usize,
    base_name: &str,
) -> Result<(usize, usize), String> {
    let mut usernames: Vec<String> = Vec::new();
    let mut skipped = 0usize;
    let mut done = 0usize;
    let limit = limit.clamp(1, 1_000_000);
    let mut iter = client.iter_messages(peer_ref).limit(limit);

    // кэш PeerId → username (сообщения часто от одних авторов)
    let mut cache: HashMap<grammers_session::types::PeerId, Option<String>> = HashMap::new();

    while let Some(m) = iter
        .next()
        .await
        .map_err(|e| format!("iter_messages: {e}"))?
    {
        done += 1;
        let username = match m.sender_id() {
            Some(pid) => {
                if let Some(cached) = cache.get(&pid) {
                    cached.clone()
                } else {
                    // async-цепочка через match (and_then с await внутри
                    // замыкания невозможен)
                    let name = {
                        let sender = m.sender_ref().await.ok().flatten();
                        if let Some(r) = sender {
                            match client.resolve_peer(r).await {
                                Ok(Peer::User(u)) => match u.username() {
                                    Some(x) if !x.is_empty() => Some(x.to_string()),
                                    _ => None,
                                },
                                _ => None,
                            }
                        } else {
                            None
                        }
                    };
                    cache.insert(pid, name.clone());
                    name
                }
            }
            None => None,
        };
        match username {
            Some(u) => usernames.push(u),
            None => skipped += 1,
        }
        if done % 25 == 0 {
            state.progress(
                "parse_start",
                serde_json::json!({ "done": done, "total": limit }),
            );
        }
    }

    let path = UserDatabase::path_for(&state.dbs_dir(), base_name);
    let saved = UserDatabase::append_unique(&path, &usernames).map_err(|e| e.to_string())?;
    log::info!("parse {base_name}: сообщений {done}, авторов с username {}, сохранено {saved}", usernames.len());
    Ok((saved, skipped))
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

    state.progress("parse_start", serde_json::json!({ "done": 0, "total": limit }));

    let (saved, skipped) =
        scrape_message_authors(state, &client, peer_ref, limit, &base_name)
            .await
            .map_err(|e| super::auth::err_json("ERROR", e))?;

    Ok(serde_json::json!({
        "saved": saved,
        "skipped": skipped,
        "base": base_name,
    }))
}
