//! Инвайт-движок: базы usernames → чат, батчами, с ротацией аккаунтов и
//! обработкой FLOOD_WAIT / PEER_FLOOD (порт логики executeBroadcast из tg-piar).

use grammers_client::client::Client;
use grammers_session::types::PeerRef;

use super::state::AppState;
use super::store::{self, UserDatabase};

/// Размер батча юзеров на один channels.inviteToChannel.
const INVITE_BATCH: usize = 10;

/// Точка входа: invite_start { chat_id, database, per_account, batch_pause_ms }.
pub async fn invite_start(
    state: &AppState,
    chat_id: i64,
    database: &str,
    per_account: usize,
    batch_pause_ms: u64,
) -> Result<serde_json::Value, serde_json::Value> {
    // целевой чат
    let chat = super::chats::find_chat(state, chat_id).ok_or_else(|| {
        super::auth::err_json("CHAT_NOT_FOUND", format!("чат {chat_id} не найден в реестре"))
    })?;
    let chat_peer: PeerRef = super::chats::chat_peer_ref(&chat)
        .map_err(|e| super::auth::err_json("ERROR", e.to_string()))?;
    let input_channel = (&chat_peer).into();

    // база usernames
    let db_path = UserDatabase::path_for(&state.dbs_dir(), database);
    if !db_path.exists() {
        return Err(super::auth::err_json(
            "DB_NOT_FOUND",
            format!("база «{database}» не найдена"),
        ));
    }
    let all: Vec<String> = {
        let set = UserDatabase::read(&db_path);
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    };
    if all.is_empty() {
        return Err(super::auth::err_json("DB_EMPTY", "база пуста"));
    }

    // подключённые пиар-аккаунты
    let clients = super::connect::connected_clients(state, "piar");
    if clients.is_empty() {
        return Err(super::auth::err_json(
            "NO_ACCOUNTS",
            "нет подключённых аккаунтов в пуле «Пиар»",
        ));
    }

    let per_account = per_account.clamp(5, 50);
    let total = all.len();
    let mut queue: Vec<String> = all;
    let mut invited = 0usize;
    let mut failed = 0usize;
    let mut restricted_accounts: Vec<String> = Vec::new();
    let mut account_round = 0usize;

    let mut report = serde_json::json!({ "total": total });

    while !queue.is_empty() && !clients.is_empty() {
        // один «сет» = per_account юзеров одним аккаунтом
        for (acc_idx, (acc_id, client)) in clients.iter().enumerate() {
            if queue.is_empty() {
                break;
            }
            let take = per_account.min(queue.len());
            let mut done_this_account = 0usize;

            while done_this_account < take && !queue.is_empty() {
                let batch: Vec<String> = queue
                    .drain(..INVITE_BATCH.min(take - done_this_account).min(queue.len()))
                    .collect();

                // разрешить usernames → InputUser (кэширует сессия клиента)
                let mut users = Vec::with_capacity(batch.len());
                let mut unresolved = 0usize;
                for username in &batch {
                    match client.resolve_username(username).await {
                        Ok(Some(peer)) => {
                            if let Some(r) = peer.to_ref().await.ok().flatten() {
                                users.push((&r).into());
                            } else {
                                unresolved += 1;
                            }
                        }
                        Ok(None) => unresolved += 1,
                        Err(_) => unresolved += 1,
                    }
                }
                if users.is_empty() {
                    failed += batch.len();
                    done_this_account += batch.len();
                    continue;
                }

                let request = grammers_tl_types::functions::channels::InviteToChannel {
                    channel: input_channel.clone(),
                    users,
                };

                match client.invoke(&request).await {
                    Ok(_) => {
                        invited += batch.len() - unresolved;
                        failed += unresolved;
                    }
                    Err(e) => {
                        // PEER_FLOOD: аккаунт в карантин
                        if e.is("PEER_FLOOD") {
                            restricted_accounts.push(acc_id.clone());
                            // вернуть юзеров в очередь и выйти с этого аккаунта
                            for u in &batch {
                                queue.push(u.clone());
                            }
                            break;
                        }
                        // FLOOD_WAIT: пережидаем value секунд и ретраим батч один раз
                        if e.is("FLOOD_WAIT") {
                            let secs = flood_wait_secs(&e).unwrap_or(30);
                            state.progress(
                                "invite_start",
                                serde_json::json!({
                                    "note": format!("FLOOD_WAIT {secs}s — ждём"),
                                    "done": invited,
                                    "failed": failed,
                                    "current_account": acc_id,
                                }),
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(secs.min(300))).await;
                            match client.invoke(&request).await {
                                Ok(_) => {
                                    invited += batch.len() - unresolved;
                                    failed += unresolved;
                                }
                                Err(_) => {
                                    failed += batch.len();
                                }
                            }
                        } else {
                            failed += batch.len();
                        }
                    }
                }

                done_this_account += batch.len();
                state.progress(
                    "invite_start",
                    serde_json::json!({
                        "done": invited,
                        "failed": failed,
                        "total": total,
                        "current_account": acc_id,
                    }),
                );
            }

            if queue.is_empty() {
                break;
            }
            // пауза между аккаунтами/сетами
            if acc_idx + 1 < clients.len() {
                tokio::time::sleep(std::time::Duration::from_millis(batch_pause_ms)).await;
            }
        }

        // отбрасываем карантинные аккаунты из списка на следующий круг
        if !restricted_accounts.is_empty() {
            account_round += 1;
            let _ = account_round;
            break_with_restricted(state, &restricted_accounts);
            break;
        }
    }

    report["invited"] = serde_json::json!(invited);
    report["failed"] = serde_json::json!(failed);
    if !restricted_accounts.is_empty() {
        report["restricted_accounts"] = serde_json::json!(restricted_accounts);
    }
    Ok(report)
}

/// Вытащить секунды из FLOOD_WAIT ошибки.
fn flood_wait_secs(e: &grammers_client::InvocationError) -> Option<u64> {
    use grammers_client::InvocationError as IE;
    if let IE::Rpc(rpc) = e {
        rpc.value.map(|v| v as u64)
    } else {
        None
    }
}

/// Пометить аккаунты ограниченными (карантин) и сохранить реестр.
fn break_with_restricted(state: &AppState, ids: &[String]) {
    {
        let mut accounts = state.accounts.write();
        for id in ids {
            if let Some(e) = accounts.get_mut(id) {
                e.record.restricted = true;
                e.live = None; // отключаем (handle дропнется)
            }
        }
    }
    super::connect::save_accounts_state(state);
}
