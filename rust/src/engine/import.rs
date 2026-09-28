//! Импорт аккаунтов: StringSession / tdata → сессия SqliteSession → аккаунт.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::{Path, PathBuf};

use grammers_session::storages::SqliteSession;
use grammers_session::SessionData;

use super::connect;
use super::state::{AccountEntry, AppState, LiveAccount};
use super::store::AccountRecord;
use super::tdata;
use super::sessions::StringSessionData;

/// Создать файл сессии из auth_key + dc (штатный путь через SessionData).
/// Возвращает путь к файлу sqlite-сессии.
pub async fn create_session_file(
    sessions_dir: &Path,
    stem: &str,
    dc: i32,
    auth_key: [u8; 256],
    ip: &str,
) -> anyhow::Result<PathBuf> {
    tokio::fs::create_dir_all(sessions_dir).await?;
    let path = sessions_dir.join(format!("{stem}.sqlite"));
    if path.exists() {
        // чужой файл не трогаем: stem выбирается с учётом занятых имён
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

/// Уникальный stem файла сессии: не должен совпадать с session_file живых
/// аккаунтов (иначе повторный импорт удалит сессию подключённого клиента).
pub fn unique_stem(state: &AppState, base: &str) -> String {
    let used: HashSet<String> = {
        let accounts = state.accounts.read();
        accounts
            .values()
            .map(|e| e.record.session_file.clone())
            .collect()
    };
    let mut stem = base.to_string();
    let mut i = 0u32;
    loop {
        if !used.contains(&format!("{stem}.sqlite")) {
            return stem;
        }
        i += 1;
        stem = format!("{base}_{i}");
    }
}

/// Импорт telethon/gramjs StringSession до конца: файл → подключение → аккаунт.
pub async fn import_session_to_account(
    state: &AppState,
    pool: &str,
    session_string: &str,
    api_id: Option<i32>,
) -> anyhow::Result<serde_json::Value> {
    let parsed = StringSessionData::decode(session_string)?;
    let base = short_key_hash(session_string);
    let stem = unique_stem(state, &base);
    let path = create_session_file(
        &state.sessions_dir(),
        &stem,
        parsed.dc_id,
        parsed.auth_key,
        &parsed.ip,
    )
    .await?;
    finalize_import(state, pool, &path, api_id).await
}

/// Импорт tdata-архива до конца: распаковка → сессия → подключение → аккаунт.
/// Временный каталог — ВНУТРИ data_dir (на Android системный /tmp недоступен).
pub async fn import_tdata_to_account(
    state: &AppState,
    pool: &str,
    zip_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let tmp_root = state.data_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_root).await?;
    let tmp = tempfile::Builder::new()
        .prefix("tdata_")
        .tempdir_in(&tmp_root)?;

    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Найти внутри запись, заканчивающуюся на tdata/key_data* (или tdata\...)
    let mut tdata_dir: Option<PathBuf> = None;
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
    archive.extract(tmp.path())?;
    let dir = tdata_dir.ok_or_else(|| {
        anyhow::anyhow!("в архиве не найдена папка tdata (ожидался key_data внутри)")
    })?;

    let info = tdata::parse_tdata(&dir)?;
    let base = format!("tdata{}", info.user_id);
    let stem = unique_stem(state, &base);
    let path = create_session_file(
        &state.sessions_dir(),
        &stem,
        info.main_dc,
        info.auth_key,
        &info.ip,
    )
    .await?;
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
    let user_id = me.id().bare_id().unwrap_or_default().to_string();
    // id записи: "{user_id}@{pool}" — аккаунт может жить в обоих пулах
    let record_id = format!("{user_id}@{pool}");
    let session_file = session_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();

    let record = AccountRecord {
        id: record_id.clone(),
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
        // legacy: запись старого формата id (просто user_id) в этом же пуле
        if let Some(old) = accounts.get(&user_id) {
            if old.record.pool == pool {
                accounts.remove(&user_id);
            }
        }
        accounts.insert(
            record_id.clone(),
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
    Ok(serde_json::json!({ "account_id": record_id, "stage": "done" }))
}

/// Короткий хеш строки для именования файлов сессий.
pub fn short_key_hash(s: &str) -> String {
    use sha1::Digest as _;
    let mut h = sha1::Sha1::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    hex::encode(&digest[..6])
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
