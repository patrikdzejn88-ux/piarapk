//! Подключение/отключение аккаунтов (SenderPool + Client).

use std::sync::Arc;

use grammers_client::client::Client;
use grammers_mtsender::{ConnectionParams, SenderPool, SenderPoolFatHandle};
use grammers_session::storages::SqliteSession;
use tokio::sync::mpsc::UnboundedReceiver;

use super::state::{AccountEntry, AppState, LiveAccount};
use super::store;

/// Пул sender'ов + клиент + задача runner'а.
pub struct StartedClient {
    pub client: Client,
    pub handle: SenderPoolFatHandle,
}

/// Запустить клиент из файла сессии (спавнит runner в текущем runtime).
pub async fn start_client(
    session_path: &std::path::Path,
    api_id: i32,
) -> anyhow::Result<StartedClient> {
    let session = Arc::new(SqliteSession::open(session_path).await?);
    let params = ConnectionParams {
        device_model: "piarapk".into(),
        system_version: "Android 14".into(),
        app_version: "0.1".into(),
        system_lang_code: "ru".into(),
        lang_code: "ru".into(),
        use_ipv6: false,
        ..Default::default()
    };
    let SenderPool {
        runner,
        handle,
        updates,
    } = SenderPool::with_configuration(session, api_id, params);
    let client = Client::new(handle.clone());
    tokio::spawn(runner.run());
    tokio::spawn(drain_updates(updates));
    Ok(StartedClient { client, handle })
}

/// Поглощать поток updates (пока не обрабатываем).
async fn drain_updates(mut rx: UnboundedReceiver<grammers_session::updates::UpdatesLike>) {
    while rx.recv().await.is_some() {
        // no-op
    }
}

/// Подключить аккаунт по id (живой клиент + статус authorized).
pub async fn connect_account(state: &AppState, id: &str) -> anyhow::Result<serde_json::Value> {
    let session_file = {
        let accounts = state.accounts.read();
        accounts
            .get(id)
            .map(|e| e.record.session_file.clone())
            .ok_or_else(|| anyhow::anyhow!("аккаунт не найден: {id}"))?
    };
    // погасить старое подключение, если было (иначе задвоение runner'ов)
    let old_handle = {
        let mut accounts = state.accounts.write();
        accounts
            .get_mut(id)
            .and_then(|e| e.live.take().map(|l| l._handle))
    };
    if let Some(h) = old_handle {
        h.quit();
    }
    let session_path = state.sessions_dir().join(&session_file);
    let started = start_client(&session_path, state.api_id).await?;
    let authorized = started.client.is_authorized().await?;
    if !authorized {
        return Err(anyhow::anyhow!("сессия не авторизована (файл повреждён/логин не завершён)"));
    }
    let me = started.client.get_me().await?;
    let (first_name, last_name, username, phone) = (
        me.first_name().unwrap_or_default().to_string(),
        me.last_name().unwrap_or_default().to_string(),
        me.username().unwrap_or_default().to_string(),
        me.phone().unwrap_or_default().to_string(),
    );
    let user_id = me.id().bare_id().unwrap_or_default();

    let mut accounts = state.accounts.write();
    if let Some(e) = accounts.get_mut(id) {
        e.record.first_name = first_name;
        e.record.last_name = last_name;
        e.record.username = username;
        e.record.phone = phone;
        e.record.restricted = false;
        e.live = Some(LiveAccount {
            client: started.client.clone(),
            _handle: started.handle,
        });
    }
    drop(accounts);
    save_accounts_state(state);

    Ok(serde_json::json!({
        "id": id,
        "user_id": user_id,
        "authorized": true,
    }))
}

/// Отключить аккаунт (завершить runner через handle.quit()).
pub async fn disconnect_account(state: &AppState, id: &str) -> anyhow::Result<()> {
    let handle = {
        let mut accounts = state.accounts.write();
        match accounts.get_mut(id) {
            Some(e) if e.live.is_some() => {
                let live = e.live.take().unwrap();
                Some(live._handle)
            }
            _ => None,
        }
    };
    if let Some(h) = handle {
        h.quit();
    }
    Ok(())
}

/// Загрузить все аккаунты при старте (без подключения).
pub fn load_entries(state: &AppState) {
    let file = store::load_accounts(&state.data_dir);
    let mut accounts = state.accounts.write();
    accounts.clear();
    for rec in file.accounts {
        accounts.insert(
            rec.id.clone(),
            AccountEntry {
                record: rec,
                live: None,
            },
        );
    }
}

/// Сохранить accounts.json из текущих записей.
pub fn save_accounts_state(state: &AppState) {
    let accounts = state.accounts.read();
    let file = store::AccountsFile {
        accounts: accounts.values().map(|e| e.record.clone()).collect(),
    };
    let _ = store::save_accounts(&state.data_dir, &file);
}

/// Первый подключённый аккаунт: сначала prefer-пул, затем любой
/// (для чатов/постинга — предпочитаем пиар-аккаунты).
pub fn connected_client(state: &AppState, prefer: &str) -> Option<Client> {
    let accounts = state.accounts.read();
    let mut any: Option<Client> = None;
    for e in accounts.values() {
        if e.live.is_some() && !e.record.restricted {
            let c = e.live.as_ref().map(|l| l.client.clone());
            if e.record.pool == prefer {
                return c;
            }
            if any.is_none() {
                any = c;
            }
        }
    }
    any
}

/// Подключённый клиент СТРОГО из указанного пула (без fallback) —
/// для парсера (изоляция пулов).
pub fn connected_client_strict(state: &AppState, pool: &str) -> Option<Client> {
    let accounts = state.accounts.read();
    for e in accounts.values() {
        if e.record.pool == pool && e.live.is_some() && !e.record.restricted {
            return e.live.as_ref().map(|l| l.client.clone());
        }
    }
    None
}

/// Подключённые клиенты пула (для инвайтов — только piar).
pub fn connected_clients(state: &AppState, pool: &str) -> Vec<(String, Client)> {
    let accounts = state.accounts.read();
    accounts
        .iter()
        .filter(|(_, e)| e.record.pool == pool && e.live.is_some() && !e.record.restricted)
        .filter_map(|(id, e)| e.live.as_ref().map(|l| (id.clone(), l.client.clone())))
        .collect()
}

/// Реестр аккаунтов → JSON для list_accounts.
pub fn accounts_json(state: &AppState) -> serde_json::Value {
    let accounts = state.accounts.read();
    let mut list: Vec<serde_json::Value> = Vec::new();
    for e in accounts.values() {
        list.push(serde_json::json!({
            "id": e.record.id,
            "phone": e.record.phone,
            "first_name": e.record.first_name,
            "last_name": e.record.last_name,
            "username": e.record.username,
            "pool": e.record.pool,
            "connected": e.live.is_some(),
            "restricted": e.record.restricted,
        }));
    }
    list.sort_by(|a, b| {
        let pool_a = a["pool"].as_str().unwrap_or("");
        let pool_b = b["pool"].as_str().unwrap_or("");
        let id_a = a["id"].as_str().unwrap_or("").parse::<i64>().unwrap_or(0);
        let id_b = b["id"].as_str().unwrap_or("").parse::<i64>().unwrap_or(0);
        pool_a.cmp(pool_b).then(id_a.cmp(&id_b))
    });
    serde_json::Value::Array(list)
}
