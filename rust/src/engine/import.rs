//! Импорт аккаунтов: StringSession / tdata → сессия SqliteSession.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::{Path, PathBuf};

use grammers_session::storages::SqliteSession;
use grammers_session::SessionData;

use super::connect;
use super::state::{AccountEntry, AppState, LiveAccount};
use super::store::AccountRecord;
use super::tdata::{self, TdataInfo};

use super::sessions::StringSessionData;

/// Создать файл сессии из auth_key + dc (штатный путь через SessionData).
/// Возвращает путь к файлу sqlite-сессии.
pub async fn create_session_file(
    sessions_dir: &std::path::Path,
    id: &str,
    dc: i32,
    auth_key: [u8; 256],
    ip: &str,
) -> anyhow::Result<PathBuf> {
    tokio::fs::create_dir_all(sessions_dir).await?;
    let path = sessions_dir.join(format!("{id}.sqlite"));
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }

    let mut data = SessionData::default();
    data.home_dc = dc;
    let opt = data
        .dc_options
        .get(&dc)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("неизвестный DC: {dc}"))?;
    let ipv4: Ipv4Addr = ip
        .parse()
        .map_err(|_| anyhow::anyhow!("не парсится IP: {ip}"))?;
    let mut opt = opt;
    opt.auth_key = Some(auth_key);
    opt.ipv4 = SocketAddrV4::new(ipv4, 443);
    data.dc_options.insert(dc, opt);

    let session = SqliteSession::open(&path).await?;
    data.import_to(&session).await?;
    Ok(path)
}

/// Импорт telethon/gramjs StringSession → файл сессии.
pub async fn import_string_session(
    sessions_dir: &std::path::Path,
    session_string: &str,
) -> anyhow::Result<(PathBuf, StringSessionData)> {
    let parsed = StringSessionData::decode(session_string)?;
    let id = short_key_hash(session_string);
    let path = create_session_file(
        sessions_dir,
        &id,
        parsed.dc_id,
        parsed.auth_key,
        &parsed.ip,
    )
    .await?;
    Ok((path, parsed))
}

/// Импорт zip-архива tdata → файл сессии.
pub async fn import_tdata_zip(
    sessions_dir: &std::path::Path,
    zip_path: &std::path::Path,
) -> anyhow::Result<(PathBuf, TdataInfo)> {
    let tmp = tempfile::tempdir()?;
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Найти внутри запись, заканчивающуюся на tdata/key_data0 (или key_data1/s)
    let mut tdata_dir: Option<std::path::PathBuf> = None;
    for i in 0..archive.len() {
        let name = archive.by_index(i)?.name().to_string();
        let norm = name.replace('\\', "/");
        if let Some(idx) = norm.find("tdata/key_data") {
            let dir = norm[..idx + "tdata".len()].to_string();
            if tdata_dir.is_none() {
                tdata_dir = Some(tmp.path().join(dir));
            }
            break;
        }
    }
    // распаковать всё
    archive.extract(tmp.path())?;
    let dir = tdata_dir.ok_or_else(|| {
        anyhow::anyhow!("в архиве не найдена папка tdata (ожидался key_data внутри)")
    })?;

    let info = tdata::parse_tdata(&dir)?;
    let id = format!("tdata{}", info.user_id);
    let path = create_session_file(
        sessions_dir,
        &id,
        info.main_dc,
        info.auth_key,
        &info.ip,
    )
    .await?;
    Ok((path, info))
}

/// Короткий хеш строки для именования временных файлов.
pub fn short_key_hash(s: &str) -> String {
    use sha1::Digest as _;
    let mut h = sha1::Sha1::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    hex::encode(&digest[..6])
}

/// Импорт StringSession до конца: файл сессии → подключение → аккаунт в пуле.
pub async fn import_session_to_account(
    state: &AppState,
    pool: &str,
    session_string: &str,
    api_id: Option<i32>,
) -> anyhow::Result<serde_json::Value> {
    let sessions_dir = state.sessions_dir();
    let (path, _parsed) = import_string_session(&sessions_dir, session_string).await?;
    finalize_import(state, pool, &path, api_id).await
}

/// Импорт tdata-архива до конца: распаковка → сессия → подключение → аккаунт.
pub async fn import_tdata_to_account(
    state: &AppState,
    pool: &str,
    zip_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let sessions_dir = state.sessions_dir();
    let (path, _info) =
        import_tdata_zip(&sessions_dir, Path::new(zip_path)).await?;
    finalize_import(state, pool, &path, None).await
}

/// Подключить импортированную сессию, проверить авторизацию, создать запись.
async fn finalize_import(
    state: &AppState,
    pool: &str,
    session_path: &Path,
    api_id: Option<i32>,
) -> anyhow::Result<serde_json::Value> {
    let started = connect::start_client(session_path, api_id.unwrap_or(state.api_id)).await?;
    let authorized = started
        .client
        .is_authorized()
        .await
        .map_err(|e| anyhow::anyhow!("проверка авторизации: {e}"))?;
    if !authorized {
        let _ = tokio::fs::remove_file(session_path).await;
        return Err(anyhow::anyhow!(
            "сессия недействительна (is_authorized=false) — вероятно, кикнута/невалидна"
        ));
    }
    let me = started.client.get_me().await?;
    let id = me.id().bare_id().unwrap_or_default().to_string();
    let session_file = session_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();

    let record = AccountRecord {
        id: id.clone(),
        phone: me.phone().unwrap_or_default().to_string(),
        first_name: me.first_name().unwrap_or_default().to_string(),
        last_name: me.last_name().unwrap_or_default().to_string(),
        username: me.username().unwrap_or_default().to_string(),
        pool: pool.to_string(),
        restricted: false,
        session_file,
        added_at: now_secs(),
    };
    {
        let mut accounts = state.accounts.write();
        accounts.insert(
            id.clone(),
            AccountEntry {
                record,
                live: Some(LiveAccount {
                    client: started.client.clone(),
                    _handle: started.handle,
                }),
            },
        );
    }
    connect::save_accounts_state(state);
    Ok(serde_json::json!({ "account_id": id, "stage": "done" }))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
