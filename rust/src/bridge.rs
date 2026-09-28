//! C-ABI мост: диспетчеризация методов, реестр async-задач, очередь событий.
//! Протокол см. lib/src/core/bridge.dart (Dart-сторона).

use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_int;
use std::sync::Arc;

use crate::engine::state::{state as current_state, AppState, STATE};
use crate::engine::{auth, chats, connect, import, inviter, scraper, store};

/// Креды приложения (выданы владельцем на my.telegram.org).
/// Переопределяются настройками приложения при желании.
pub const DEFAULT_API_ID: i32 = 28614298;
const DEFAULT_API_HASH: &str = "e482876dfc8d703565d3261bdd7d4bde";

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

/// Простейший логгер: log-записи ядра уходят в очередь событий (type:"log"),
/// чтобы их было видно в приложении (диагностика «молчаливых» проблем).
struct EventLogger;

impl log::Log for EventLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if let Some(app) = current_state() {
            let ev = serde_json::json!({
                "request_id": 0,
                "type": "log",
                "method": "core",
                "ok": true,
                "data": {
                    "level": record.level().to_string(),
                    "target": record.target(),
                    "message": record.args().to_string(),
                },
                "error": null,
            });
            app.events.push(ev.to_string());
        }
    }

    fn flush(&self) {}
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
        parser_cancel: std::sync::atomic::AtomicBool::new(false),
    };
    connect::load_entries(&app);

    let _ = STATE.set(Arc::new(app));
    // Паники в spawn-задачах больше не молчат — уходят в лог-события
    std::panic::set_hook(Box::new(|info| {
        if let Some(app) = current_state() {
            let ev = serde_json::json!({
                "request_id": 0,
                "type": "log",
                "method": "panic",
                "ok": false,
                "data": {"level": "panic", "target": "", "message": info.to_string()},
                "error": null,
            });
            app.events.push(ev.to_string());
        }
    }));
    let _ = log::set_boxed_logger(Box::new(EventLogger));
    log::set_max_level(log::LevelFilter::Info);
    if api_id == DEFAULT_API_ID {
        log::info!("piarcore: используем штатную пару приложения (api_id={api_id})");
    } else {
        log::info!("piarcore: используется своя пара api_id={api_id}");
    }
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
        "get_database" => {
            // содержимое базы (для экспорта на устройство)
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let path = store::UserDatabase::path_for(&app.dbs_dir(), &store::sanitize_name(&name));
            match std::fs::read_to_string(&path) {
                Ok(content) => serde_json::json!({ "ok": true, "data": { "name": name, "content": content } }),
                Err(e) => serde_json::json!({ "ok": false, "error": { "code": "DB_READ", "message": e.to_string() } }),
            }
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
    let panic_app = app.clone();
    let m2 = m.clone();
    // Паника внутри задачи НЕ должна глотать result-событие (иначе Dart-Future
    // висит вечно) — ловим unwind и отправляем ошибку PANIC
    let fut = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
        async move {
            let (ok, data, error) = dispatch_async(&task_app, &m, &params).await;
            task_app.event(id, "result", &m, ok, data, error);
        },
    ));
    app.runtime.spawn(async move {
        if let Err(p) = fut.await {
            log::error!("паника в задаче {m2}: {p:?}");
            panic_app.event(
                id,
                "result",
                &m2,
                false,
                serde_json::Value::Null,
                serde_json::json!({ "code": "PANIC", "message": format!("{p:?}") }),
            );
        }
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
            map_anyhow(delete_account(app, &id).await.map(|_| serde_json::json!({"deleted": true})))
        }
        // ---- чаты ----
        "add_chat" => {
            let link = str_param(params, "link", "");
            map_anyhow(chats::add_chat(app, &link).await)
        }
        "post_message" => {
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64()).unwrap_or_default();
            let text = str_param(params, "text", "");
            let image_path = str_param(params, "image_path", "");
            map_anyhow(chats::post_message(app, chat_id, &text, &image_path).await)
        }
        "create_channel" => {
            let title = str_param(params, "title", "Новый канал");
            let about = str_param(params, "about", "");
            map_anyhow(chats::create_readonly_channel(app, &title, &about).await)
        }
        // ---- парсер (только по сообщениям чата) ----
        "parse_start" => {
            let chat = str_param(params, "chat", "");
            let dialog_id = params.get("dialog_id").and_then(|v| v.as_i64()).unwrap_or_default();
            let access_hash = params.get("access_hash").and_then(|v| v.as_i64()).unwrap_or_default();
            let title = str_param(params, "title", "");
            let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10000) as usize;
            log::info!("parse_start: чат {chat}, диалог {dialog_id}, лимит {limit}");
            scraper::parse_start(app, &chat, dialog_id, access_hash, &title, limit).await
        }
        // ---- чаты аккаунта парсера (его диалоги) ----
        "list_account_chats" => {
            chats::list_account_chats(app).await
        }
        // ---- пиар (один аккаунт, N людей из базы) ----
        "invite_start" => {
            let chat_id = params.get("chat_id").and_then(|v| v.as_i64()).unwrap_or_default();
            let database = str_param(params, "database", "");
            let message = str_param(params, "message", "");
            let image_path = str_param(params, "image_path", "");
            let count = params.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            log::info!("invite_start: чат {chat_id}, база {database}, людей {count}");
            inviter::invite_start(app, chat_id, &database, &message, &image_path, count).await
        }
        // ---- управление парсингом/базами ----
        "parse_cancel" => {
            app.request_parser_cancel();
            log::info!("parse_cancel: запрошена остановка парсинга");
            Ok(serde_json::json!({"cancel_requested": true}))
        }
        "delete_database" => {
            let name = str_param(params, "name", "");
            match store::delete_database(&app.dbs_dir(), &name) {
                Ok(true) => Ok(serde_json::json!({"deleted": true})),
                Ok(false) => Err(auth::err_json("DB_NOT_FOUND", format!("база «{name}» не найдена"))),
                Err(e) => Err(auth::err_json("ERROR", e.to_string())),
            }
        }
        "add_to_database" => {
            let name = str_param(params, "name", "");
            let raw = str_param(params, "usernames", "");
            let usernames: Vec<String> = raw
                .split(|c: char| c == '\n' || c == ',' || c == ' ')
                .map(|s| s.trim().trim_start_matches('@').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if usernames.is_empty() {
                Err(auth::err_json("EMPTY", "список usernames пуст"))
            } else {
                let path =
                    store::UserDatabase::path_for(&app.dbs_dir(), &store::sanitize_name(&name));
                match store::UserDatabase::append_unique(&path, &usernames) {
                    Ok(added) => Ok(serde_json::json!({"added": added})),
                    Err(e) => Err(auth::err_json("ERROR", e.to_string())),
                }
            }
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

/// Удалить аккаунт: запись, live-клиент и файл сессии (если им не пользуется
/// другая запись — например, тот же аккаунт во втором пуле).
async fn delete_account(app: &Arc<AppState>, id: &str) -> anyhow::Result<()> {
    let _ = connect::disconnect_account(app, id).await;
    let session_file = {
        let mut accounts = app.accounts.write();
        accounts.remove(id).map(|e| e.record.session_file)
    };
    if let Some(f) = session_file {
        let used_elsewhere = {
            let accounts = app.accounts.read();
            accounts.values().any(|e| e.record.session_file == f)
        };
        if !used_elsewhere {
            let _ = std::fs::remove_file(app.sessions_dir().join(f));
        }
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
