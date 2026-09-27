//! piarcore — Telegram multi-account core (grammers 0.10) с C-ABI JSON-мостом.
//!
//! Структура:
//! - `bridge`  — C-ABI (piar_init/piar_call/piar_call_async/piar_poll/...),
//!   диспетчеризация методов, реестр async-задач, очередь событий.
//! - `engine`  — вся логика: аккаунты, авторизация, импорт сессий (StringSession/
//!   tdata), чаты, инвайты, парсер, базы пользователей.
//!
//! Компиляция и тесты — в CI (Codemagic); локально на dev-машине Rust не
//! собирается (Smart App Control блокирует билд-скрипты).

mod bridge;
pub mod engine;

/// Номер версии ABI-протокола (для ping).
pub const ABI_VERSION: &str = "0.1";
