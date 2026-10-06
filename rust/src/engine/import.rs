//! Импорт аккаунтов: StringSession / tdata → сессия SqliteSession → аккаунт.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6};
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
    addr: &str,
    port: u16,
) -> anyhow::Result<PathBuf> {
    tokio::fs::create_dir_all(sessions_dir).await?;
    let path = sessions_dir.join(format!("{stem}.sqlite"));
    // B.2.6: НЕ удаляем существующий файл — он может принадлежать уже
    // открытой сессии. Имя выбирается уникальным (unique_stem); если файл
    // всё же существует, это гонка/чужой файл — сообщаем об ошибке.
    if path.exists() {
        anyhow::bail!(
            "файл сессии {} уже существует — повторите импорт",
            path.display()
        );
    }

    let mut data = SessionData::default();
    data.home_dc = dc;
    let opt = data
        .dc_options
        .get(&dc)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("неизвестный DC: {dc}"))?;
    let mut opt = opt;
    opt.auth_key = Some(auth_key);
    let port = if port == 0 { 443 } else { port };
    // B.3.3: поддержка SocketAddr (v4/v6) и проброс порта
    match addr.parse::<SocketAddr>() {
        Ok(SocketAddr::V4(v4)) => opt.ipv4 = v4,
        Ok(SocketAddr::V6(v6)) => opt.ipv6 = v6,
        Err(_) => {
            let ip: IpAddr = addr
                .parse()
                .map_err(|_| anyhow::anyhow!("не парсится IP: {addr}"))?;
            match ip {
                IpAddr::V4(v4) => opt.ipv4 = SocketAddrV4::new(v4, port),
                IpAddr::V6(v6) => opt.ipv6 = SocketAddrV6::new(v6, port, 0, 0),
            }
        }
    }
    data.dc_options.insert(dc, opt);

    let session = SqliteSession::open(&path).await?;
    data.import_to(&session).await?;
    Ok(path)
}

/// B.3.3: корректный порт из данных сессии (0/вне диапазона → 443).
fn to_port(p: i32) -> u16 {
    if (1..=65535).contains(&p) {
        p as u16
    } else {
        443
    }
}

/// Уникальный stem файла сессии: не должен совпадать с session_file живых
/// аккаунтов (иначе повторный импорт удалит сессию подключённого клиента).
pub fn unique_stem(state: &AppState, base: &str) -> String {
    let mut used: HashSet<String> = {
        let accounts = state.accounts.read();
        accounts
            .values()
            .map(|e| e.record.session_file.clone())
            .collect()
    };
    // B.2.6: учитываем и незавершённые входы (pending_auths), иначе имя
    // может совпасть с файлом полу-созданной сессии.
    {
        let pending = state.pending_auths.lock();
        for a in pending.values() {
            if let Ok(p) = a.try_lock() {
                used.insert(p.session_file.clone());
            }
        }
    }
    // B.2.6: случайный суффикс исключает TOCTOU-гонку при выборе имени
    let mut stem = format!("{base}_{}", unique_suffix());
    let mut i = 0u32;
    while used.contains(&format!("{stem}.sqlite")) {
        i += 1;
        stem = format!("{base}_{}_{i}", unique_suffix());
    }
    stem
}

/// B.2.6: короткий случайный суффикс (наносекунды + счётчик).
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{t:x}{n:x}")
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
        to_port(parsed.port),
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

    // B.3.2: распаковка — блокирующий IO, выносим в spawn_blocking,
    // с лимитами размера/числа записей и явной проверкой enclosed_name.
    let zip_path_owned = zip_path.to_string();
    let tmp_path = tmp.path().to_path_buf();
    let tdata_dir = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<PathBuf>> {
        const MAX_ENTRIES: usize = 10_000;
        const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
        const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

        let file = std::fs::File::open(&zip_path_owned)?;
        let mut archive = zip::ZipArchive::new(file)?;
        if archive.len() > MAX_ENTRIES {
            anyhow::bail!("в архиве слишком много записей: {}", archive.len());
        }
        // Найти внутри запись, заканчивающуюся на tdata/key_data* (или tdata\...)
        let mut tdata_dir: Option<PathBuf> = None;
        let mut total: u64 = 0;
        for i in 0..archive.len() {
            let entry = archive.by_index(i)?;
            // явная защита от zip-slip
            let Some(rel) = entry.enclosed_name() else {
                anyhow::bail!("небезопасный путь в архиве: {}", entry.name());
            };
            let norm = rel.to_string_lossy().replace('\\', "/");
            if tdata_dir.is_none() {
                if let Some(idx) = norm.find("tdata/key_data") {
                    tdata_dir = Some(tmp_path.join(&norm[..idx + "tdata".len()]));
                }
            }
            if entry.size() > MAX_ENTRY_BYTES {
                anyhow::bail!("запись архива слишком велика: {}", entry.name());
            }
            total = total.saturating_add(entry.size());
            if total > MAX_TOTAL_BYTES {
                anyhow::bail!("распакованный архив превышает лимит размера");
            }
        }
        archive.extract(&tmp_path)?;
        Ok(tdata_dir)
    })
    .await??;
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
        to_port(info.port),
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
    // B.1.4: на error-путях гасим клиент до возврата; B.2.2 — под таймаутом
    let authorized = match connect::with_rpc_timeout(started.client.is_authorized()).await {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => {
            started.handle.quit();
            return Err(anyhow::anyhow!("проверка авторизации: {e}"));
        }
        Err(_) => {
            started.handle.quit();
            return Err(anyhow::anyhow!(
                "таймаут сети ({} с) при проверке авторизации",
                connect::RPC_TIMEOUT.as_secs()
            ));
        }
    };
    if !authorized {
        // B.1.4: сначала гасим клиент, только потом удаляем файл сессии
        started.handle.quit();
        let _ = tokio::fs::remove_file(session_path).await;
        return Err(anyhow::anyhow!(
            "сессия недействительна (is_authorized=false) — вероятно, кикнута/невалидна"
        ));
    }
    let me = match connect::with_rpc_timeout(started.client.get_me()).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            started.handle.quit();
            return Err(anyhow::anyhow!("get_me: {e}"));
        }
        Err(_) => {
            started.handle.quit();
            return Err(anyhow::anyhow!(
                "таймаут сети ({} с) при get_me",
                connect::RPC_TIMEOUT.as_secs()
            ));
        }
    };
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
        // legacy: запись старого формата id (просто user_id) в этом же пуле.
        // B.1.4: старый live-клиент гасим, иначе утечка соединения.
        if let Some(old) = accounts.remove(&user_id) {
            if old.record.pool == pool {
                if let Some(l) = old.live {
                    l._handle.quit();
                }
            } else {
                accounts.insert(user_id.clone(), old);
            }
        }
        // insert поверх существующей записи: старый live тоже гасим (B.1.4)
        if let Some(old) = accounts.remove(&record_id) {
            if let Some(l) = old.live {
                l._handle.quit();
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
