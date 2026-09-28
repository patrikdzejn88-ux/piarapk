//! Глобальное состояние ядра.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use grammers_client::client::{Client, LoginToken, PasswordToken};
use grammers_mtsender::SenderPoolFatHandle;
use parking_lot::{Mutex, RwLock};
use tokio::runtime::Runtime;

use super::store::{AccountRecord, ChatsFile};

/// Очередь событий для поллера (Dart вызывает piar_poll каждые ~100 мс).
/// Каждый элемент — готовая JSON-строка события.
#[derive(Default)]
pub struct EventQueue {
    queue: Mutex<VecDeque<String>>,
}

impl EventQueue {
    pub fn push(&self, event_json: String) {
        let mut q = self.queue.lock();
        q.push_back(event_json);
        // защита от переполнения, если поллер Dart надолго встал (бэкграунд)
        if q.len() > 2000 {
            q.pop_front();
        }
    }

    /// Забрать все накопленные события (JSON-массив).
    /// try_lock: если очередь внезапно заблокирована (авария в другой
    /// задаче), поллер НЕ должен вечно висеть и морозить UI-поток Dart.
    pub fn drain(&self) -> String {
        let Ok(mut q) = self.queue.try_lock() else {
            return "[]".to_string();
        };
        if q.is_empty() {
            return "[]".to_string();
        }
        let drained: Vec<String> = q.drain(..).collect();
        format!("[{}]", drained.join(","))
    }
}

/// Подключённый Telegram-клиент аккаунта.
pub struct LiveAccount {
    pub client: Client,
    /// Handle пула отправителей — хранится, чтобы уметь корректно завершить.
    pub _handle: SenderPoolFatHandle,
}

/// Запись об аккаунте: постоянная часть (store) + живой клиент.
pub struct AccountEntry {
    pub record: AccountRecord,
    pub live: Option<LiveAccount>,
}

/// Незавершённый вход по номеру (телефон → код → 2FA).
pub struct PendingAuth {
    pub phone: String,
    pub pool: String,
    pub client: Client,
    pub _handle: SenderPoolFatHandle,
    pub login_token: LoginToken,
    pub password_token: Option<PasswordToken>,
    /// Имя файла сессии (в sessions/) — фиксируется при создании попытки.
    pub session_file: String,
}

/// Глобальное состояние ядра.
pub struct AppState {
    pub data_dir: PathBuf,
    pub api_id: i32,
    pub api_hash: String,
    pub runtime: Arc<Runtime>,
    pub events: EventQueue,
    pub accounts: RwLock<HashMap<String, AccountEntry>>,
    pub pending_auths: Mutex<HashMap<String, Arc<tokio::sync::Mutex<PendingAuth>>>>,
    pub next_request_id: AtomicU64,
    pub chats: Mutex<ChatsFile>,
    pub started_at: std::time::Instant,
}

impl AppState {
    pub fn sessions_dir(&self) -> PathBuf {
        self.data_dir.join("sessions")
    }

    pub fn dbs_dir(&self) -> PathBuf {
        self.data_dir.join("dbs")
    }

    /// Записать событие в очередь (type: result/progress/log).
    pub fn event(
        &self,
        request_id: u64,
        event_type: &str,
        method: &str,
        ok: bool,
        data: serde_json::Value,
        error: serde_json::Value,
    ) {
        let ev = serde_json::json!({
            "request_id": request_id,
            "type": event_type,
            "method": method,
            "ok": ok,
            "data": data,
            "error": error,
        });
        self.events.push(ev.to_string());
    }

    /// Прогресс-событие (без request_id — широковещательное).
    pub fn progress(&self, method: &str, data: serde_json::Value) {
        let ev = serde_json::json!({
            "request_id": 0,
            "type": "progress",
            "method": method,
            "ok": true,
            "data": data,
            "error": null,
        });
        self.events.push(ev.to_string());
    }

    pub fn next_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }
}

/// Глобальный экземпляр состояния (инициализируется в piar_init).
pub static STATE: std::sync::OnceLock<Arc<AppState>> = std::sync::OnceLock::new();

/// Доступ к состоянию; None до piar_init.
pub fn state() -> Option<Arc<AppState>> {
    STATE.get().cloned()
}
