//! Авторизация по номеру: телефон → код → (2FA) → аккаунт.

use std::sync::Arc;

use grammers_client::client::{LoginToken, SignInError};
use grammers_client::client::Client;
use grammers_mtsender::{ConnectionParams, SenderPool};
use grammers_session::storages::SqliteSession;

use super::connect::save_accounts_state;
use super::state::{AccountEntry, AppState, LiveAccount, PendingAuth};

/// Отправить код на телефон (создаёт временную сессию и PendingAuth).
/// Повторный вызов гасит прошлую попытку и отправляет НОВЫЙ код.
pub async fn add_account_phone(
    state: &AppState,
    pool: &str,
    phone: &str,
) -> anyhow::Result<serde_json::Value> {
    // погасить прошлую попытку (тот же телефон)
    if let Some(old) = state.pending_auths.lock().remove(phone) {
        let mut p = old.lock().await;
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
    if session_path.exists() {
        let _ = tokio::fs::remove_file(&session_path).await;
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
    tokio::spawn(runner.run());
    tokio::spawn(async move {
        use tokio::sync::mpsc::UnboundedReceiver;
        let mut rx: UnboundedReceiver<grammers_session::updates::UpdatesLike> = updates;
        while rx.recv().await.is_some() {}
    });

    let login_token: LoginToken = client
        .request_login_code(phone, &state.api_hash)
        .await
        .map_err(|e| anyhow::anyhow!("не удалось отправить код: {e}"))?;

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
        })),
    );
    Ok(serde_json::json!({ "stage": "code" }))
}

/// Ввести код из Telegram.
pub async fn submit_auth_code(
    state: &AppState,
    phone: &str,
    code: &str,
) -> Result<serde_json::Value, serde_json::Value> {
    let pending = {
        let map = state.pending_auths.lock();
        map.get(phone).cloned()
    };
    let Some(pending) = pending else {
        return Err(err_json("NO_AUTH", "сначала запросите код (add_account_phone)"));
    };
    let mut p = pending.lock().await;

    match p.client.sign_in(&p.login_token, code).await {
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
    phone: &str,
    password: &str,
) -> Result<serde_json::Value, serde_json::Value> {
    let pending = {
        let map = state.pending_auths.lock();
        map.get(phone).cloned()
    };
    let Some(pending) = pending else {
        return Err(err_json("NO_AUTH", "нет начатого входа для этого номера"));
    };
    let mut p = pending.lock().await;
    // PasswordToken не Clone — забираем (на InvalidPassword вернётся новый)
    let Some(token) = p.password_token.take() else {
        return Err(err_json("NO_NEED_2FA", "этот вход не ждёт 2FA-пароль (введите код)"));
    };
    match p.client.check_password(token, password.as_bytes()).await {
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

    let entry = AccountEntry {
        record: super::store::AccountRecord {
            id: id.clone(),
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
        accounts.insert(id.clone(), entry);
    }
    state.pending_auths.lock().remove(&p.phone);
    save_accounts_state(state);

    serde_json::json!({ "stage": "done", "account_id": id })
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
