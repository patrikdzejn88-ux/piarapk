//! piarcore — C-ABI JSON-мост для piarapk (фаза заглушки).
//!
//! Полное ядро (grammers: аккаунты, авторизация, инвайты, парсер) добавляется
//! в src/core и мостится через этот же ABI. Компиляция — в CI (Codemagic),
//! локально Rust не собирается (Smart App Control блокирует билд-скрипты).
//!
//! Протокол (используется Dart-стороной lib/src/core/bridge.dart):
//! - piar_init(config_json)            {"data_dir": "..."}
//! - piar_call(method, params, out)     синхронные вызовы → {"ok":bool,"data":..,"error":..}
//! - piar_call_async(method, params, &request_id) → события через piar_poll
//! - piar_poll(timeout_ms, out)         → [{"request_id":n,"type":"result"|"progress"|"log",
//!                                          "method":"..","ok":true,"data":{...},"error":null}]
//! - piar_free(ptr) / piar_shutdown()

use std::collections::VecDeque;
use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_int;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static EVENTS: Lazy<Mutex<VecDeque<String>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static CONFIG: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

fn take_cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn write_out(out: *mut *mut c_char, s: String) -> c_int {
    match (CString::new(s), out.is_null()) {
        (Ok(cs), false) => {
            unsafe { *out = cs.into_raw() };
            0
        }
        _ => -1,
    }
}

/// Прочитать строку params как JSON; в фазе заглушки не разбирается.
#[allow(dead_code)]
fn params_or_empty(p: *const c_char) -> String {
    let s = take_cstr(p);
    if s.is_empty() {
        "{}".to_string()
    } else {
        s
    }
}

#[no_mangle]
pub extern "C" fn piar_init(config_json: *const c_char) -> c_int {
    *CONFIG.lock().unwrap() = Some(take_cstr(config_json));
    0
}

#[no_mangle]
pub extern "C" fn piar_call(
    method: *const c_char,
    params_json: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    let m = take_cstr(method);
    let _p = params_or_empty(params_json);
    let resp = match m.as_str() {
        "ping" => r#"{"ok":true,"data":{"version":"stub","crate":"piarcore"}}"#.to_string(),
        "list_accounts" => r#"{"ok":true,"data":[]}"#.to_string(),
        other => format!(
            r#"{{"ok":false,"error":"unknown method (stub phase): {other}"}}"#
        ),
    };
    write_out(out, resp)
}

#[no_mangle]
pub extern "C" fn piar_call_async(
    method: *const c_char,
    params_json: *const c_char,
    request_id: *mut u64,
) -> c_int {
    let m = take_cstr(method);
    let _p = params_or_empty(params_json);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if !request_id.is_null() {
        unsafe { *request_id = id };
    }
    let ev = format!(
        r#"{{"request_id":{id},"type":"result","method":"{m}","ok":true,"data":{{"stub":true}},"error":null}}"#
    );
    EVENTS.lock().unwrap().push_back(ev);
    0
}

#[no_mangle]
pub extern "C" fn piar_poll(_timeout_ms: c_int, out: *mut *mut c_char) -> c_int {
    let drained: Vec<String> = EVENTS.lock().unwrap().drain(..).collect();
    let s = if drained.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", drained.join(","))
    };
    write_out(out, s)
}

#[no_mangle]
pub extern "C" fn piar_free(p: *mut c_char) {
    if !p.is_null() {
        unsafe { drop(CString::from_raw(p)) };
    }
}

#[no_mangle]
pub extern "C" fn piar_shutdown() {
    // фаза заглушки: no-op
}

// Проверка линковки в тестах CI: cargo test
#[cfg(test)]
mod tests {
    #[test]
    fn stub_smoke() {
        assert_eq!(2 + 2, 4);
    }
}
