//! Авторизация по номеру: телефон → код → (2FA) → аккаунт.

use std::sync::Arc;
use std::time::{Duration, Instant};

use grammers_client::client::{LoginToken, SignInError};
use grammers_client::client::Client;
use grammers_mtsender::{ConnectionParams, SenderPool};
use grammers_session::storages::SqliteSession;

use super::connect::save_accounts_state;
use super::state::{AccountEntry, AppState, LiveAccount, PendingAuth};

/// Отправить код на телефон (создаёт временную сессию и PendingAuth).
/// Повторный вызов гасит прошлую попытку и отправляет НОВЫЙ код.
/// Номер нормализуется: мусор-символы вон, '+' подставляется автоматически.
pub async fn add_account_phone(
    state: &AppState,
    pool: &str,
    raw_phone: &str,
) -> anyhow::Result<serde_json::Value> {
    let phone = normalize_phone(raw_phone);
    // B.3.1: телефон — PII, в логи только маска
    let masked = mask_phone(&phone);
    log::info!("add_account_phone: {masked} (пул {pool})");
    // погасить прошлую попытку (тот же телефон).
    // ВАЖНО: parking_lot-гард не должен переживать .await (future: Send),
    // поэтому блокировка строго в scoped-блоке до await.
    let old = {
        let mut map = state.pending_auths.lock();
        map.remove(phone.as_str())
    };
    if let Some(old) = old {
        let p = old.lock().await;
        p._handle.quit();
        let file = p.session_file.clone();
        let still_used = {
            let accounts = state.accounts.read();
            accounts
                .values()
                .any(|e| e.record.session_file == file)
        };
        if !still_used {
            let _ = tokio::fs::remove_file(state.sessions_dir().join(&file)).await;
        }
    }

    let sessions_dir = state.sessions_dir();
    tokio::fs::create_dir_all(&sessions_dir).await?;
    let base = super::import::short_key_hash(&format!("{pool}:{phone}"));
    let stem = super::import::unique_stem(state, &format!("pending_{base}"));
    let session_file = format!("{stem}.sqlite");
    let session_path = sessions_dir.join(&session_file);
    // B.2.6: не удаляем чужой файл сессии без проверки владельца —
    // имя уже уникально (unique_stem), существование = гонка/чужой файл
    if session_path.exists() {
        anyhow::bail!(
            "файл сессии {} уже существует — повторите запрос кода",
            session_path.display()
        );
    }
    let session = Arc::new(SqliteSession::open(&session_path).await?);
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
    } = SenderPool::with_configuration(session, state.api_id, params);
    let client = Client::new(handle.clone());
    // исход runner'а не игнорируем: его смерть (например, сеть недоступна)
    // логируется, а не молчит
    let runner_handle = tokio::spawn(runner.run());
    tokio::spawn(async move {
        if let Err(e) = runner_handle.await {
            log::error!("sender-pool runner паника: {e}");
        }
    });
    tokio::spawn(async move {
        use tokio::sync::mpsc::UnboundedReceiver;
        let mut rx: UnboundedReceiver<grammers_session::updates::UpdatesLike> = updates;
        while rx.recv().await.is_some() {}
    });

    log::info!("запрашиваем код для {masked}");
    // таймаут сети: без него недоступная сеть = вечный спиннер вместо ошибки
    let login_token: LoginToken = match tokio::time::timeout(
        Duration::from_secs(30),
        client.request_login_code(phone.as_str(), &state.api_hash),
    )
    .await
    {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => {
            // B.1.4/B.2.1: полу-созданный SenderPool гасим и убираем файл сессии
            handle.quit();
            let _ = tokio::fs::remove_file(&session_path).await;
            return Err(anyhow::anyhow!("не удалось отправить код: {e}"));
        }
        Err(_) => {
            handle.quit();
            let _ = tokio::fs::remove_file(&session_path).await;
            return Err(anyhow::anyhow!(
                "таймаут сети (30 с) — проверьте интернет-соединение"
            ));
        }
    };
    log::info!("код отправлен для {masked}");

    state.pending_auths.lock().insert(
        phone.to_string(),
        Arc::new(tokio::sync::Mutex::new(PendingAuth {
            phone: phone.to_string(),
            pool: pool.to_string(),
            client,
            _handle: handle,
            login_token,
            password_token: None,
            session_file: session_file.clone(),
            created_at: Instant::now(),
        })),
    );
    Ok(serde_json::json!({ "stage": "code" }))
}

/// Нормализация телефона: мусор-символы вон, '+' подставляется автоматически.
/// Применяется ВО ВСЕХ методах авторизации — ключ реестра PendingAuth один
/// и тот же независимо от того, как пользователь ввёл номер.
fn normalize_phone(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| *c != ' ' && *c != '-' && *c != '(' && *c != ')' && *c != '\u{a0}')
        .collect();
    if cleaned.starts_with('+') {
        cleaned
    } else {
        format!("+{cleaned}")
    }
}

/// B.3.1: маска телефона (PII) для логов: `+79***67`.
fn mask_phone(phone: &str) -> String {
    let chars: Vec<char> = phone.chars().collect();
    if chars.len() <= 4 {
        return "***".to_string();
    }
    let head: String = chars.iter().take(3).collect();
    let tail: String = chars[chars.len() - 2..].iter().collect();
    format!("{head}***{tail}")
}

/// TTL заброшенной попытки входа (B.2.1): 10 минут.
pub const PENDING_AUTH_TTL: Duration = Duration::from_secs(600);
/// Период прохода janitor'а (B.2.1).
const PENDING_AUTH_SWEEP: Duration = Duration::from_secs(60);

/// Фоновый janitor: периодически гасит заброшенные PendingAuth (B.2.1).
pub fn spawn_pending_auth_janitor(state: &Arc<AppState>) {
    let app = state.clone();
    state.runtime.spawn(async move {
        loop {
            tokio::time::sleep(PENDING_AUTH_SWEEP).await;
            reap_expired_pending_auths(&app, PENDING_AUTH_TTL).await;
        }
    });
}

/// Погасить и удалить PendingAuth старше `ttl` (B.2.1), чтобы
/// `pending_auths` не рос неограниченно.
pub async fn reap_expired_pending_auths(state: &AppState, ttl: Duration) {
    let now = Instant::now();
    let expired: Vec<Arc<tokio::sync::Mutex<PendingAuth>>> = {
        let mut map = state.pending_auths.lock();
        let keys: Vec<String> = map.keys().cloned().collect();
        let mut expired = Vec::new();
        for k in keys {
            let is_expired = match map.get(&k) {
                Some(a) => match a.try_lock() {
                    Ok(p) => now.saturating_duration_since(p.created_at) >= ttl,
                    // занят другой задачей — проверим в следующем проходе
                    Err(_) => false,
                },
                None => false,
            };
            if is_expired {
                if let Some(a) = map.remove(&k) {
                    expired.push(a);
                }
            }
        }
        expired
    };
    for a in expired {
        let p = a.lock().await;
        p._handle.quit();
        let file = p.session_file.clone();
        drop(p);
        let still_used = {
            let accounts = state.accounts.read();
            accounts.values().any(|e| e.record.session_file == file)
        };
        if !still_used {
            let _ = tokio::fs::remove_file(state.sessions_dir().join(&file)).await;
        }
        log::info!("pending_auths: истёкшая попытка входа погашена и удалена");
    }
}

/// Ввести код из Telegram.
pub async fn submit_auth_code(
    state: &AppState,
    raw_phone: &str,
    code: &str,
) -> Result<serde_json::Value, serde_json::Value> {
    let phone = normalize_phone(raw_phone);
    let pending = {
        let map = state.pending_auths.lock();
        map.get(phone.as_str()).cloned()
    };
    let Some(pending) = pending else {
        return Err(err_json("NO_AUTH", "начала запросите код заново (кнопка «Отправить код») и введите его"));
    };
    let mut p = pending.lock().await;

    let sign_result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        p.client.sign_in(&p.login_token, code),
    )
    .await;
    let sign_result = match sign_result {
        Ok(r) => r,
        Err(_) => {
            return Err(err_json("TIMEOUT", "таймаут сети (30 с) при входе — проверьте интернет"))
        }
    };
    match sign_result {
        Ok(user) => Ok(finalize_login(state, &mut p, user).await),
        Err(SignInError::PasswordRequired(token)) => {
            p.password_token = Some(token);
            Err(err_json("NEED_2FA", "у аккаунта включена 2FA — введите пароль"))
        }
        Err(SignInError::SignUpRequired) => {
            Err(err_json("SIGNUP_REQUIRED", "номер не зарегистрирован в Telegram"))
        }
        Err(SignInError::InvalidCode) => Err(err_json("INVALID_CODE", "неверный код")),
        Err(SignInError::InvalidPassword(token)) => {
            p.password_token = Some(token);
            Err(err_json("NEED_2FA", "неверный 2FA-пароль — попробуйте ещё раз"))
        }
        Err(e) => Err(err_json("ERROR", format!("ошибка входа: {e}"))),
    }
}

/// Ввести 2FA-пароль.
pub async fn submit_auth_password(
    state: &AppState,
    raw_phone: &str,
    password: &str,
) -> Result<serde_json::Value, serde_json::Value> {
    let phone = normalize_phone(raw_phone);
    let pending = {
        let map = state.pending_auths.lock();
        map.get(phone.as_str()).cloned()
    };
    let Some(pending) = pending else {
        return Err(err_json("NO_AUTH", "нет начатого входа для этого номера — запросите код заново"));
    };
    let mut p = pending.lock().await;
    // PasswordToken не Clone — забираем (на InvalidPassword вернётся новый)
    let Some(token) = p.password_token.take() else {
        return Err(err_json("NO_NEED_2FA", "этот вход не ждёт 2FA-пароль (введите код)"));
    };
    let check_result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        p.client.check_password(token, password.as_bytes()),
    )
    .await;
    let check_result = match check_result {
        Ok(r) => r,
        Err(_) => {
            return Err(err_json("TIMEOUT", "таймаут сети (30 с) при проверке 2FA"))
        }
    };
    match check_result {
        Ok(user) => Ok(finalize_login(state, &mut p, user).await),
        Err(SignInError::InvalidPassword(token)) => {
            p.password_token = Some(token);
            Err(err_json("NEED_2FA", "неверный 2FA-пароль — попробуйте ещё раз"))
        }
        Err(e) => Err(err_json("ERROR", format!("ошибка 2FA: {e}"))),
    }
}

/// Завершить вход: сохранить аккаунт, живой клиент переезжает в пул.
async fn finalize_login(
    state: &AppState,
    p: &mut PendingAuth,
    user: grammers_client::peer::User,
) -> serde_json::Value {
    let id = user
        .id()
        .bare_id()
        .unwrap_or_default()
        .to_string();
    let phone = user.phone().unwrap_or(&p.phone).to_string();
    let first_name = user.first_name().unwrap_or_default().to_string();
    let last_name = user.last_name().unwrap_or_default().to_string();
    let username = user.username().unwrap_or_default().to_string();

    // файл сессии: тот, что создан при add_account_phone (фиксирован в PendingAuth)
    let session_file = p.session_file.clone();

    // id записи: "{user_id}@{pool}" — ОДИН и тот же аккаунт может жить в обоих
    // пулах (два логина = две авторизации, как два устройства). Заодно убираем
    // legacy-запись со старым форматом id (просто user_id), если она в этом пуле.
    let legacy_id = id.clone();
    let entry = AccountEntry {
        record: super::store::AccountRecord {
            id: format!("{id}@{pool2}", id = id, pool2 = p.pool),
            phone,
            first_name,
            last_name,
            username,
            pool: p.pool.clone(),
            restricted: false,
            session_file,
            added_at: chrono_now(),
        },
        live: Some(LiveAccount {
            client: p.client.clone(),
            _handle: p._handle.clone(),
        }),
    };

    {
        let mut accounts = state.accounts.write();
        // legacy: запись старого формата (id = просто user_id) в этом же пуле.
        // B.1.4: старый live-клиент гасим, иначе утечка соединения.
        if let Some(old) = accounts.remove(&legacy_id) {
            if old.record.pool == p.pool {
                if let Some(l) = old.live {
                    l._handle.quit();
                }
            } else {
                accounts.insert(legacy_id.clone(), old);
            }
        }
        // insert поверх существующей записи: старый live тоже гасим (B.1.4)
        if let Some(old) = accounts.remove(&entry.record.id) {
            if let Some(l) = old.live {
                l._handle.quit();
            }
        }
        accounts.insert(entry.record.id.clone(), entry);
    }
    state.pending_auths.lock().remove(&p.phone);
    save_accounts_state(state);

    serde_json::json!({ "stage": "done", "account_id": format!("{id}@{}", p.pool) })
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Ошибка в формате моста: {"code": "...", "message": "..."}.
pub fn err_json(code: &str, message: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "code": code, "message": message.into() })
}
