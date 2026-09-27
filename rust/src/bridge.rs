//! C-ABI мост: диспетчеризация методов, реестр async-задач, очередь событий.
//! Протокол см. lib/src/core/bridge.dart (Dart-сторона).

use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_int;
use std::sync::Arc;

use crate::engine::state::{state as current_state, AppState, STATE};
use crate::engine::{auth, chats, connect, import, inviter, scraper, store};

/// Публичные креды Telegram Desktop (как в старом tg-piar); переопределяются
/// конфигом piar_init.
const DEFAULT_API_ID: i32 = 2040;
const DEFAULT_API_HASH: &str = "b18441a1ff6077106a04f1e0a0f9fb";

fn take_cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn write_out(out: *mut *mut c_char, s: String) -> c_int {
    if out.is_null() {
        return -1;
    }
    match CString::new(s) {
        Ok(cs) => {
            unsafe { *out = cs.into_raw() };
            0
        }
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn piar_init(config_json: *const c_char) -> c_int {
    if STATE.get().is_some() {
        return 0;
    }
    let cfg_raw = take_cstr(config_json);
    let cfg: serde_json::Value = serde_json::from_str(&cfg_raw).unwrap_or_default();
    let data_dir = cfg
        .get("data_dir")
        .and_then(|v| v.as_str())
        .unwrap_or("data")
        .to_string();
    let api_id = cfg
        .get("api_id")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
        .unwrap_or(DEFAULT_API_ID);
    let api_hash = cfg
        .get("api_hash")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_API_HASH)
        .to_string();

    let data_dir = std::path::PathBuf::from(data_dir);
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::create_dir_all(data_dir.join("sessions"));
    let _ = std::fs::create_dir_all(data_dir.join("dbs"));

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime"),
    );

    let app = AppState {
        chats: parking_lot::Mutex::new(store::load_chats(&data_dir)),
        data_dir,
        api_id,
        api_hash,
        runtime,
        events: Default::default(),
        accounts: Default::default(),
        pending_auths: Default::default(),
        next_request_id: Default::default(),
        started_at: std::time::Instant::now(),
    };
    connect::load_entries(&app);

    let _ = STATE.set(Arc::new(app));
    0
}

/// Синхронные методы: быстрые операции без сети.
#[no_mangle]
pub extern "C" fn piar_call(
    method: *const c_char,
    params_json: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    let m = take_cstr(method);
    let p_raw = take_cstr(params_json);
    let params: serde_json::Value = serde_json::from_str(&p_raw).unwrap_or_default();

    let Some(app) = current_state() else {
        return write_out(
            out,
            r#"{"ok":false,"error":{"code":"NO_INIT","message":"piar_init не вызван"}}"#.into(),
        );
    };

    let resp = match m.as_str() {
        "ping" => serde_json::json!({
            "ok": true,
            "data": { "version": crate::ABI_VERSION, "uptime_secs": app.started_at.elapsed().as_secs() },
        }),
        "list_accounts" => serde_json::json!({
            "ok": true,
            "data": connect::accounts_json(&app),
        }),
        "list_chats" => serde_json::json!({
            "ok": true,
            "data": chats::chats_json(&app),
        }),
        "list_databases" => {
            let dbs = store::list_dbs(&app.dbs_dir());
            let list: Vec<serde_json::Value> = dbs
                .into_iter()
                .map(|(name, count)| serde_json::json!({ "name": name, "entries": count }))
                .collect();
            serde_json::json!({ "ok": true, "data": list })
        }
        other => serde_json::json!({
            "ok": false,
            "error": { "code": "UNKNOWN_METHOD", "message": format!("неизвестный sync-метод: {other}") },
        }),
    };
    let _ = params;
    write_out(out, resp.to_string())
}

/// Асинхронные методы: сеть/тяжёлые операции, ответ — событием result.
#[no_mangle]
pub extern "C" fn piar_call_async(
    method: *const c_char,
    params_json: *const c_char,
    request_id: *mut u64,
) -> c_int {
    let m = take_cstr(method);
    let p_raw = take_cstr(params_json);
    let params: serde_json::Value = serde_json::from_str(&p_raw).unwrap_or_default();

    let Some(app) = current_state() else {
        return -1;
    };
    let id = app.next_id();
    if !request_id.is_null() {
        unsafe { *request_id = id };
    }

    let task_app = app.clone();
    app.runtime.spawn(async move {
        let (ok, data, error) = dispatch_async(&task_app, &m, &params).await;
        task_app.event(id, "result", &m, ok, data, error);
    });
    0
}

async fn dispatch_async(
    app: &Arc<AppState>,
    method: &str,
    params: &serde_json::Value,
) -> (bool, serde_json::Value, serde_json::Value) {
    log::debug!("async call: {method} {params}");
    let result: Result<serde_json::Value, serde_json::Value> = match method {
        // ---- аккаунты ----
        "add_account_phone" => {
            let pool = str_param(params, "pool", "piar");
            let phone = str_param(params, "phone", "");
            map_anyhow(auth::add_account_phone(app, &pool, &phone).await)
        }
        "submit_auth_code" => {
            let phone = str_param(params, "phone", "");
            let code = str_param(params, "code", "");
            auth::submit_auth_code(app, &phone, &code).await
        }
        "submit_auth_password" => {
            let phone = str_param(params, "phone", "");
            let password = str_param(params, "password", "");
            auth::submit_auth_password(app, &phone, &password).await
        }
        "import_string_session" => {
            let pool = str_param(params, "pool", "piar");
            let session = str_param(params, "session", "");
            let api_id = params.get("api_id").and_then(|v| v.as_i64()).map(|v| v as i32);
            map_anyhow(import::import_session_to_account(app, &pool, &session, api_id).await)
        }
        "import_tdata" => {
            let pool = str_param(params, "pool", "piar");
            let zip_path = str_param(params, "zip_path", "");
            map_anyhow(import::import_tdata_to_account(app, &pool, &zip_path).await)
        }
        "connect_account" => {
            let id = str_param(params, "id", "");
            map_anyhow(connect::connect_account(app, &id).await)
        }
        "disconnect_account" => {
            let id = str_param(params, "id", "");
            map_anyhow(
                connect::disconnect_account(app, &id)
                    .await
                    .map(|_| serde_json::json!({"disconnected": true})),
            )
        }
        "delete_account" => {
            let id = str_param(params, "id", "");
            map_anyhow(delete_account(app, &id).map(|_| serde_json::json!({"deleted": true})))
        }
        // ---- чаты ----
        "add_chat" => {
            let link = str_param(params, "link", "");
            map_anyhow(chats::add_chat(app, &link).await)
        }
        "post_message" => {
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64()).unwrap_or_default();
            let text = str_param(params, "text", "");
            map_anyhow(chats::post_message(app, chat_id, &text).await)
        }
        "create_channel" => {
            let title = str_param(params, "title", "Новый канал");
            let about = str_param(params, "about", "");
            map_anyhow(chats::create_readonly_channel(app, &title, &about).await)
        }
        // ---- парсер ----
        "parse_start" => {
            let chat = str_param(params, "chat", "");
            let mode = str_param(params, "mode", "participants");
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(1000) as usize;
            scraper::parse_start(app, &chat, &mode, limit).await
        }
        // ---- пиар ----
        "invite_start" => {
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64()).unwrap_or_default();
            let database = str_param(params, "database", "");
            let message = str_param(params, "message", "");
            let per_account = params.get("per_account").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let pause = params.get("batch_pause_ms").and_then(|v| v.as_u64()).unwrap_or(5000);
            inviter::invite_start(app, chat_id, &database, &message, per_account, pause).await
        }
        other => Err(auth::err_json(
            "UNKNOWN_METHOD",
            format!("неизвестный async-метод: {other}"),
        )),
    };
    match result {
        Ok(data) => (true, data, serde_json::Value::Null),
        Err(error) => (false, serde_json::Value::Null, error),
    }
}

fn str_param(params: &serde_json::Value, key: &str, default: &str) -> String {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

fn map_anyhow(
    r: anyhow::Result<serde_json::Value>,
) -> Result<serde_json::Value, serde_json::Value> {
    r.map_err(|e| auth::err_json("ERROR", e.to_string()))
}

/// Удалить аккаунт: запись, live-клиент и файл сессии.
fn delete_account(app: &Arc<AppState>, id: &str) -> anyhow::Result<()> {
    let _ = connect::disconnect_account(app, id).await;
    let session_file = {
        let mut accounts = app.accounts.write();
        accounts.remove(id).map(|e| e.record.session_file)
    };
    if let Some(f) = session_file {
        let _ = std::fs::remove_file(app.sessions_dir().join(f));
    }
    connect::save_accounts_state(app);
    Ok(())
}

#[no_mangle]
pub extern "C" fn piar_poll(_timeout_ms: c_int, out: *mut *mut c_char) -> c_int {
    let Some(app) = current_state() else {
        return write_out(out, "[]".to_string());
    };
    write_out(out, app.events.drain())
}

#[no_mangle]
pub extern "C" fn piar_free(p: *mut c_char) {
    if !p.is_null() {
        unsafe { drop(CString::from_raw(p)) };
    }
}

#[no_mangle]
pub extern "C" fn piar_shutdown() {
    if let Some(app) = current_state() {
        // корректно завершить все клиенты
        let handles: Vec<_> = {
            let accounts = app.accounts.write();
            accounts
                .values()
                .filter_map(|e| e.live.as_ref().map(|l| l._handle.clone()))
                .collect()
        };
        for h in handles {
            h.quit();
        }
    }
}
