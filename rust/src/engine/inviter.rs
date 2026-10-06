//! Инвайт-движок: ОДИН аккаунт (владелец чата) добавляет людей из базы
//! в чат батчами с паузами, FLOOD_WAIT пережидается, PEER_FLOOD — карантин
//! аккаунта с остановкой (ротации нет — аккаунт один). После инвайтов
//! отправляется сообщение в чат.

use std::time::Duration;

use grammers_session::types::PeerRef;

use super::state::AppState;
use super::store::UserDatabase;

/// Размер батча юзеров на один channels.inviteToChannel.
const INVITE_BATCH: usize = 10;

/// Пауза между батчами (мс), по образцу эталона (2 с).
const BATCH_PAUSE_MS: u64 = 2000;

/// Точка входа: invite_start { chat_id, database, message, count }.
/// count = 0 → вся база. Работает первым подключённым пиар-аккаунтом.
pub async fn invite_start(
    state: &AppState,
    chat_id: i64,
    database: &str,
    message: &str,
    image_path: &str,
    count: usize,
) -> Result<serde_json::Value, serde_json::Value> {
    // целевой чат
    let chat = super::chats::find_chat(state, chat_id).ok_or_else(|| {
        super::auth::err_json("CHAT_NOT_FOUND", format!("чат {chat_id} не найден в реестре"))
    })?;
    let chat_peer: PeerRef = super::chats::chat_peer_ref(&chat)
        .map_err(|e| super::auth::err_json("ERROR", e.to_string()))?;
    let input_channel: grammers_tl_types::enums::InputChannel = (&chat_peer).into();

    // единственный аккаунт — первый подключённый из пула «Пиар»
    let clients = super::connect::connected_clients(state, "piar");
    let Some((acc_id, client)) = clients.first().cloned() else {
        return Err(super::auth::err_json(
            "NO_ACCOUNTS",
            "нет подключённых аккаунтов в пуле «Пиар» — добавьте и подключите аккаунт в разделе «Аккаунты» (вкладка «Пиар»)",
        ));
    };
    log::info!("invite: аккаунт {acc_id}, чат {chat_id}");

    // база usernames
    let db_path = UserDatabase::path_for(&state.dbs_dir(), database);
    if !db_path.exists() {
        return Err(super::auth::err_json(
            "DB_NOT_FOUND",
            format!("база «{database}» не найдена"),
        ));
    }
    let mut all: Vec<String> = {
        let set = UserDatabase::read(&db_path);
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    };
    if all.is_empty() {
        return Err(super::auth::err_json("DB_EMPTY", "база пуста"));
    }
    let total = if count == 0 { all.len() } else { count.min(all.len()) };
    all.truncate(total);

    let mut invited = 0usize;
    let mut failed = 0usize;
    let mut stopped_reason: Option<String> = None;
    let mut log: Vec<String> = Vec::new();

    while !all.is_empty() {
        // B.2.4: инвайт можно отменить тем же флагом, что и парсинг
        if state.parser_cancelled() {
            let reason = "остановлено пользователем".to_string();
            push_log(&mut log, reason.clone());
            stopped_reason.get_or_insert(reason);
            break;
        }
        let batch: Vec<String> = all
            .drain(..INVITE_BATCH.min(all.len()))
            .collect();

        // разрешить usernames → InputUser (кэширует сессия клиента)
        let mut users: Vec<grammers_tl_types::enums::InputUser> = Vec::with_capacity(batch.len());
        let mut unresolved = 0usize;
        for username in &batch {
            // B.2.2: резолв username — сетевой RPC под единым таймаутом
            match super::connect::with_rpc_timeout(client.resolve_username(username)).await {
                Ok(Ok(Some(peer))) => {
                    if let Some(r) = peer.to_ref().await.ok().flatten() {
                        users.push((&r).into());
                    } else {
                        unresolved += 1;
                        push_log(&mut log, format!("@{username}: нет peer ref"));
                    }
                }
                Ok(Ok(None)) => {
                    unresolved += 1;
                    push_log(&mut log, format!("@{username}: не найден"));
                }
                Ok(Err(e)) => {
                    unresolved += 1;
                    push_log(&mut log, format!("@{username}: {e}"));
                }
                Err(_) => {
                    unresolved += 1;
                    push_log(&mut log, format!("@{username}: таймаут сети"));
                }
            }
        }
        // B.2.4: отмена во время резолва usernames
        if state.parser_cancelled() {
            stopped_reason.get_or_insert("остановлено пользователем".to_string());
            break;
        }
        if users.is_empty() {
            failed += batch.len();
            emit_invite_progress(state, invited, failed, total, &log, None);
            if !cancellable_sleep(state, Duration::from_millis(BATCH_PAUSE_MS)).await {
                stopped_reason.get_or_insert("остановлено пользователем".to_string());
                break;
            }
            continue;
        }

        let request = grammers_tl_types::functions::channels::InviteToChannel {
            channel: input_channel.clone(),
            users,
        };

        // B.2.2: инвайт — сетевой RPC под единым таймаутом
        match super::connect::with_rpc_timeout(client.invoke(&request)).await {
            Ok(Ok(_)) => {
                invited += batch.len() - unresolved;
                failed += unresolved;
            }
            Ok(Err(e)) if e.is("PEER_FLOOD") => {
                // аккаунт-wide флуд: карантин аккаунта и остановка
                quarantine_accounts(state, &[acc_id.clone()]);
                let reason = format!("PEER_FLOOD: аккаунт {acc_id} отправлен в карантин");
                push_log(&mut log, reason.clone());
                stopped_reason.get_or_insert(reason);
                break;
            }
            Ok(Err(e)) if e.is("FLOOD_WAIT") => {
                let secs = flood_wait_secs(&e).unwrap_or(30).min(300);
                log::warn!("invite: FLOOD_WAIT {secs}s — ждём");
                push_log(&mut log, format!("FLOOD_WAIT {secs}s — пережидаем"));
                emit_invite_progress(
                    state,
                    invited,
                    failed,
                    total,
                    &log,
                    Some(format!("FLOOD_WAIT {secs}s — пережидаем")),
                );
                // B.2.4: FLOOD_WAIT-пауза прерывается отменой
                if !cancellable_sleep(state, Duration::from_secs(secs)).await {
                    stopped_reason.get_or_insert("остановлено пользователем".to_string());
                    break;
                }
                // B.2.2: повтор после FLOOD_WAIT — тоже под таймаутом
                match super::connect::with_rpc_timeout(client.invoke(&request)).await {
                    Ok(Ok(_)) => {
                        invited += batch.len() - unresolved;
                        failed += unresolved;
                    }
                    Ok(Err(e2)) if e2.is("PEER_FLOOD") => {
                        quarantine_accounts(state, &[acc_id.clone()]);
                        let reason = format!("PEER_FLOOD: аккаунт {acc_id} в карантине");
                        push_log(&mut log, reason.clone());
                        stopped_reason.get_or_insert(reason);
                        break;
                    }
                    Ok(Err(e2)) => {
                        failed += batch.len();
                        push_log(&mut log, format!("инвайт после FLOOD_WAIT: {e2}"));
                    }
                    Err(_) => {
                        failed += batch.len();
                        push_log(
                            &mut log,
                            "инвайт после FLOOD_WAIT: таймаут сети".to_string(),
                        );
                    }
                }
            }
            Ok(Err(e)) => {
                failed += batch.len();
                push_log(&mut log, format!("инвайт отклонён: {e}"));
            }
            Err(_) => {
                failed += batch.len();
                push_log(&mut log, "инвайт: таймаут сети".to_string());
            }
        }

        emit_invite_progress(state, invited, failed, total, &log, None);

        if !all.is_empty()
            && !cancellable_sleep(state, Duration::from_millis(BATCH_PAUSE_MS)).await
        {
            stopped_reason.get_or_insert("остановлено пользователем".to_string());
            break;
        }
    }

    let mut report = serde_json::json!({
        "total": total,
        "invited": invited,
        "failed": failed,
        "remaining": all.len(),
        "log": log,
    });
    // сообщение (текст + опционально картинка) в чат после инвайтов
    if (!message.trim().is_empty() || !image_path.trim().is_empty())
        && stopped_reason.is_none()
    {
        match super::chats::post_message(state, chat_id, message, image_path).await {
            Ok(_) => report["message_sent"] = serde_json::json!(true),
            Err(e) => report["message_error"] = serde_json::json!(e.to_string()),
        }
    }
    if let Some(reason) = stopped_reason {
        report["stopped_reason"] = serde_json::json!(reason);
    }
    log::info!("invite: готово — {report}");
    Ok(report)
}

/// B.2.4: прерываемый сон — проверяет отмену парсера/инвайта каждые 200 мс.
/// Возвращает `false`, если поступил запрос отмены.
async fn cancellable_sleep(state: &AppState, dur: Duration) -> bool {
    let step = Duration::from_millis(200);
    let mut left = dur;
    while !left.is_zero() {
        if state.parser_cancelled() {
            return false;
        }
        let s = step.min(left);
        tokio::time::sleep(s).await;
        left = left.saturating_sub(s);
    }
    !state.parser_cancelled()
}

fn push_log(log: &mut Vec<String>, line: String) {
    log.push(line);
    if log.len() > 40 {
        log.remove(0);
    }
}

fn emit_invite_progress(
    state: &AppState,
    invited: usize,
    failed: usize,
    total: usize,
    log: &[String],
    note: Option<String>,
) {
    let mut payload = serde_json::json!({
        "done": invited,
        "failed": failed,
        "total": total,
        "log": log,
    });
    if let Some(note) = note {
        payload["note"] = serde_json::json!(note);
    }
    state.progress("invite_start", payload);
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
fn quarantine_accounts(state: &AppState, ids: &[String]) {
    {
        let mut accounts = state.accounts.write();
        for id in ids {
            if let Some(e) = accounts.get_mut(id) {
                e.record.restricted = true;
                // B.1.4: перед сбросом live вызываем quit(), иначе утечка
                // соединения и «лишнее устройство» в Telegram
                if let Some(l) = e.live.take() {
                    l._handle.quit();
                }
            }
        }
    }
    super::connect::save_accounts_state(state);
}
