//! Парсер: сбор участников/авторов сообщений чата → база usernames.
//! Порт scrape-логики tg-piar (участники + авторы, дедуп, прогресс).

use grammers_client::client::Client;
use grammers_client::peer::Peer;
use grammers_session::types::PeerRef;

use super::state::AppState;
use super::store::{sanitize_name, UserDatabase};

/// Сбор участников чата (iter_participants).
async fn scrape_participants(
    state: &AppState,
    client: &Client,
    peer_ref: PeerRef,
    limit: usize,
    base_name: &str,
) -> Result<(usize, usize), String> {
    let mut usernames: Vec<String> = Vec::new();
    let mut skipped = 0usize;
    let mut iter = client.iter_participants(peer_ref);
    let total = iter.total().await.unwrap_or(0);
    let mut done = 0usize;
    while let Some(p) = iter
        .next()
        .await
        .map_err(|e| format!("iter_participants: {e}"))?
    {
        done += 1;
        let user = p.user;
        match user.username() {
            Some(u) if !u.is_empty() => usernames.push(u.to_string()),
            _ => skipped += 1,
        }
        if done % 50 == 0 {
            state.progress(
                "parse_start",
                serde_json::json!({ "done": done, "total": total }),
            );
        }
        if limit > 0 && done >= limit {
            break;
        }
    }
    let path = UserDatabase::path_for(&state.dbs_dir(), base_name);
    let saved = UserDatabase::append_unique(&path, &usernames).map_err(|e| e.to_string())?;
    Ok((saved, skipped))
}

/// Сбор авторов последних сообщений (iter_messages + resolve отправителей).
async fn scrape_message_authors(
    state: &AppState,
    client: &Client,
    peer_ref: PeerRef,
    limit: usize,
    base_name: &str,
) -> Result<(usize, usize), String> {
    let mut usernames: Vec<String> = Vec::new();
    let mut skipped = 0usize;
    let mut iter = client.iter_messages(peer_ref).limit(limit.max(1));
    let mut done = 0usize;
    // кэш: PeerId → username (много сообщений от одних авторов)
    let mut cache: std::collections::HashMap<grammers_session::types::PeerId, Option<String>> =
        std::collections::HashMap::new();
    while let Some(m) = iter
        .next()
        .await
        .map_err(|e| format!("iter_messages: {e}"))?
    {
        done += 1;
        let sender_id = m.sender_id();
        if let Some(pid) = sender_id {
            let username = if let Some(cached) = cache.get(&pid) {
                cached.clone()
            } else {
                let resolved = m
                    .sender_ref()
                    .await
                    .ok()
                    .flatten()
                    .and_then(|r| {
                        // resolve_peer может вернуть Peer::User → username
                        Some(r)
                    });
                let name = match resolved {
                    Some(r) => match client.resolve_peer(r).await {
                        Ok(Peer::User(u)) => u.username().map(|s| s.to_string()),
                        _ => None,
                    },
                    None => None,
                };
                cache.insert(pid, name.clone());
                name
            };
            match username {
                Some(u) if !u.is_empty() => usernames.push(u),
                _ => skipped += 1,
            }
        } else {
            skipped += 1;
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
    Ok((saved, skipped))
}

/// Точка входа: parse_start { chat, mode: participants|messages, limit }.
pub async fn parse_start(
    state: &AppState,
    chat: &str,
    mode: &str,
    limit: usize,
) -> Result<serde_json::Value, serde_json::Value> {
    // парсер работает ТОЛЬКО на пуле «parser» (строгая изоляция, без fallback)
    let Some(client) = super::connect::connected_client_strict(state, "parser") else {
        return Err(super::auth::err_json(
            "NO_PARSER_ACCOUNTS",
            "нет подключённых аккаунтов в пуле «Парсер» — добавьте/подключите",
        ));
    };
    let peer: Peer = client
        .resolve_username(&super::chats::normalize_chat_link(chat))
        .await
        .map_err(|e| super::auth::err_json("ERROR", format!("ошибка поиска: {e}")))?
        .ok_or_else(|| {
            super::auth::err_json("CHAT_NOT_FOUND", "чат не найден (resolve_username=None)")
        })?;
    let peer_ref = peer
        .to_ref()
        .await
        .map_err(|e| super::auth::err_json("ERROR", format!("to_ref: {e}")))?
        .ok_or_else(|| super::auth::err_json("ERROR", "пустой PeerRef"))?;

    let base_name = sanitize_name(&super::chats::normalize_chat_link(chat));

    state.progress("parse_start", serde_json::json!({ "done": 0, "total": 0 }));

    let (saved, skipped) = match mode {
        "messages" => scrape_message_authors(state, &client, peer_ref, limit, &base_name)
            .await
            .map_err(|e| super::auth::err_json("ERROR", e))?,
        _ => scrape_participants(state, &client, peer_ref, limit, &base_name)
            .await
            .map_err(|e| super::auth::err_json("ERROR", e))?,
    };

    Ok(serde_json::json!({
        "saved": saved,
        "skipped": skipped,
        "base": base_name,
    }))
}
