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

/// Единый таймаут сетевых RPC (B.2.2): 45 с.
pub const RPC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// Ошибка истечения таймаута RPC (B.2.2).
#[derive(Debug)]
pub struct RpcTimeout;

impl std::fmt::Display for RpcTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "таймаут сети ({} с)", RPC_TIMEOUT.as_secs())
    }
}

impl std::error::Error for RpcTimeout {}

/// Обернуть RPC-фьючер в единый таймаут (B.2.2), чтобы мёртвая сеть
/// не держала задачу бесконечно.
pub async fn with_rpc_timeout<F, T>(fut: F) -> Result<T, RpcTimeout>
where
    F: std::future::Future<Output = T>,
{
    match tokio::time::timeout(RPC_TIMEOUT, fut).await {
        Ok(v) => Ok(v),
        Err(_) => Err(RpcTimeout),
    }
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
    // исход runner'а логируется, а не выбрасывается
    let runner_handle = tokio::spawn(runner.run());
    tokio::spawn(async move {
        if let Err(e) = runner_handle.await {
            log::error!("sender-pool runner паника: {e}");
        }
    });
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
    // B.1.4: на error-путях после start_client гасим клиент, иначе утечка
    // соединения; B.2.2: RPC под единым таймаутом.
    let authorized = match with_rpc_timeout(started.client.is_authorized()).await {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => {
            started.handle.quit();
            return Err(anyhow::anyhow!("проверка авторизации: {e}"));
        }
        Err(_) => {
            started.handle.quit();
            return Err(anyhow::anyhow!(
                "таймаут сети ({} с) при проверке авторизации",
                RPC_TIMEOUT.as_secs()
            ));
        }
    };
    if !authorized {
        started.handle.quit();
        return Err(anyhow::anyhow!("сессия не авторизована (файл повреждён/логин не завершён)"));
    }
    let me = match with_rpc_timeout(started.client.get_me()).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            started.handle.quit();
            return Err(anyhow::anyhow!("get_me: {e}"));
        }
        Err(_) => {
            started.handle.quit();
            return Err(anyhow::anyhow!(
                "таймаут сети ({} с) при get_me",
                RPC_TIMEOUT.as_secs()
            ));
        }
    };
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

/// Перенести аккаунт в другой пул («piar» ↔ «parser»).
/// Файл сессии не трогается, подключение сохраняется.
pub fn move_account(
    state: &AppState,
    id: &str,
    to_pool: &str,
) -> anyhow::Result<serde_json::Value> {
    if to_pool != "piar" && to_pool != "parser" {
        return Err(anyhow::anyhow!("неизвестный пул: {to_pool}"));
    }
    let (mut record, live) = {
        let mut accounts = state.accounts.write();
        let Some(entry) = accounts.get_mut(id) else {
            return Err(anyhow::anyhow!("аккаунт не найден: {id}"));
        };
        (entry.record.clone(), entry.live.take())
    };
    if record.pool == to_pool {
        // вернуть live обратно
        let mut accounts = state.accounts.write();
        if let Some(e) = accounts.get_mut(id) {
            e.live = live;
        }
        return Ok(serde_json::json!({"moved": false, "reason": "уже в этом пуле"}));
    }
    let old_id = record.id.clone();
    let uid = record.id.split('@').next().unwrap_or(&record.id).to_string();
    record.pool = to_pool.to_string();
    record.id = format!("{uid}@{to_pool}");
    let new_id = record.id.clone();
    let new_session_file = record.session_file.clone();
    let mut replaced_file: Option<String> = None;
    {
        let mut accounts = state.accounts.write();
        // если в целевом пуле уже есть запись этого же аккаунта —
        // погасить её live-клиент, иначе утечка соединения
        if let Some(old) = accounts.get_mut(&new_id) {
            if let Some(l) = old.live.take() {
                l._handle.quit();
            }
            // B.3.4: файл сессии перезаписываемой записи выпадает из
            // реестра — удалим его ниже, если он не совпадает с новым
            replaced_file = Some(old.record.session_file.clone());
        }
        accounts.remove(&old_id);
        accounts.insert(new_id.clone(), AccountEntry { record, live });
    }
    // B.3.4: удаляем осиротевший файл, если он отличается от нового и
    // больше никем не используется
    if let Some(old_file) = replaced_file {
        if old_file != new_session_file {
            let still_used = {
                let accounts = state.accounts.read();
                accounts.values().any(|e| e.record.session_file == old_file)
            };
            if !still_used {
                let _ = std::fs::remove_file(state.sessions_dir().join(old_file));
            }
        }
    }
    save_accounts_state(state);
    log::info!("move_account: {old_id} → {new_id}");
    Ok(serde_json::json!({"moved": true, "id": new_id}))
}

/// Первый подключённый аккаунт: сначала prefer-пул, затем любой
/// (для чатов/постинга — предпочитаем пиар-аккаунты).
/// try_read: UI-поток не должен блокироваться даже при битой блокировке.
pub fn connected_client(state: &AppState, prefer: &str) -> Option<Client> {
    let accounts = state.accounts.try_read()?;
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
    let accounts = state.accounts.try_read()?;
    for e in accounts.values() {
        if e.record.pool == pool && e.live.is_some() && !e.record.restricted {
            return e.live.as_ref().map(|l| l.client.clone());
        }
    }
    None
}

/// Подключённые клиенты пула (для инвайтов — только piar).
pub fn connected_clients(state: &AppState, pool: &str) -> Vec<(String, Client)> {
    let Some(accounts) = state.accounts.try_read() else {
        return Vec::new();
    };
    accounts
        .iter()
        .filter(|(_, e)| e.record.pool == pool && e.live.is_some() && !e.record.restricted)
        .filter_map(|(id, e)| e.live.as_ref().map(|l| (id.clone(), l.client.clone())))
        .collect()
}

/// Реестр аккаунтов → JSON для list_accounts.
/// try_read: вызывается из UI-потока (piar_call) — не блокировать его.
pub fn accounts_json(state: &AppState) -> serde_json::Value {
    let Some(accounts) = state.accounts.try_read() else {
        log::error!("accounts: реестр заблокирован (try_read) — возвращаю пусто");
        return serde_json::Value::Array(vec![]);
    };
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
        // id теперь "{user_id}@{pool}" — берём числовую часть до '@'
        let id_a = a["id"]
            .as_str()
            .unwrap_or("")
            .split('@')
            .next()
            .unwrap_or("")
            .parse::<i64>()
            .unwrap_or(0);
        let id_b = b["id"]
            .as_str()
            .unwrap_or("")
            .split('@')
            .next()
            .unwrap_or("")
            .parse::<i64>()
            .unwrap_or(0);
        pool_a.cmp(pool_b).then(id_a.cmp(&id_b))
    });
    serde_json::Value::Array(list)
}
