//! Хранилища на диске: accounts.json, chats.json, базы пользователей.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Запись аккаунта в accounts.json.
#[derive(Serialize, Deserialize, Clone)]
pub struct AccountRecord {
    /// Telegram user id (число, строкой).
    pub id: String,
    pub phone: String,
    pub first_name: String,
    pub last_name: String,
    pub username: String,
    /// "piar" | "parser"
    pub pool: String,
    /// Аккаунт в карантине (PEER_FLOOD и т.п.).
    pub restricted: bool,
    /// Имя файла сессии в sessions/ (без каталога).
    pub session_file: String,
    /// Отметка времени добавления (unix сек).
    pub added_at: i64,
}

/// Чат (цель инвайтов / источник парсинга) в chats.json.
#[derive(Serialize, Deserialize, Clone)]
pub struct ChatRecord {
    /// Bot API dialog id: канал = -(1e12 + id).
    pub dialog_id: i64,
    /// access_hash канала.
    pub access_hash: i64,
    pub title: String,
    pub members: i64,
    pub added_at: i64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ChatsFile {
    pub chats: Vec<ChatRecord>,
}

/// accounts.json целиком.
#[derive(Serialize, Deserialize, Default)]
pub struct AccountsFile {
    pub accounts: Vec<AccountRecord>,
}

/// Загрузить accounts.json (пусто, если файла нет).
pub fn load_accounts(data_dir: &Path) -> AccountsFile {
    let path = data_dir.join("accounts.json");
    if !path.exists() {
        return AccountsFile::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Атомарно сохранить accounts.json.
pub fn save_accounts(data_dir: &Path, file: &AccountsFile) -> std::io::Result<()> {
    let path = data_dir.join("accounts.json");
    let tmp = data_dir.join("accounts.json.tmp");
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(&tmp, serde_json::to_vec_pretty(file).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// Загрузить chats.json.
pub fn load_chats(data_dir: &Path) -> ChatsFile {
    let path = data_dir.join("chats.json");
    if !path.exists() {
        return ChatsFile::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Атомарно сохранить chats.json.
pub fn save_chats(data_dir: &Path, file: &ChatsFile) -> std::io::Result<()> {
    let path = data_dir.join("chats.json");
    let tmp = data_dir.join("chats.json.tmp");
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(&tmp, serde_json::to_vec_pretty(file).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// База пользователей: текстовый файл usernames (по одному в строке).
pub struct UserDatabase {
    pub path: PathBuf,
    pub name: String,
}

impl UserDatabase {
    pub fn path_for(dbs_dir: &Path, name: &str) -> PathBuf {
        dbs_dir.join(format!("{}.txt", sanitize_name(name)))
    }

    /// Прочитать базу (HashSet lowercase).
    pub fn read(path: &Path) -> HashSet<String> {
        let mut set = HashSet::new();
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                let l = line.trim().trim_start_matches('@').to_lowercase();
                if !l.is_empty() {
                    set.insert(l);
                }
            }
        }
        set
    }

    /// Дописать usernames (без дубликатов с существующими), вернуть сколько добавлено.
    pub fn append_unique(path: &Path, usernames: &[String]) -> std::io::Result<usize> {
        let mut existing = Self::read(path);
        let mut added = 0usize;
        let mut out = String::new();
        for u in usernames {
            let l = u.trim().trim_start_matches('@').to_lowercase();
            if l.is_empty() || existing.contains(&l) {
                continue;
            }
            existing.insert(l.clone());
            out.push_str(&l);
            out.push('\n');
            added += 1;
        }
        if added > 0 {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
            f.write_all(out.as_bytes())?;
        }
        Ok(added)
    }
}

/// Список баз: [{name, entries}].
pub fn list_dbs(dbs_dir: &Path) -> Vec<(String, usize)> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dbs_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("txt") {
                let name = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                let count = UserDatabase::read(&p).len();
                result.push((name, count));
            }
        }
    }
    result.sort();
    result
}

/// Имя файла базы из ссылки/имени чата.
pub fn sanitize_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch.to_ascii_lowercase());
        }
    }
    if out.is_empty() {
        out = "db".to_string();
    }
    if out.len() > 60 {
        out.truncate(60);
    }
    out
}

/// Удалить базу по имени. Ok(false) — не существует.
pub fn delete_database(dbs_dir: &Path, name: &str) -> std::io::Result<bool> {
    let path = UserDatabase::path_for(dbs_dir, &sanitize_name(name));
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path).map(|_| true)
}

/// Создать базу с начальным списком usernames (без дубликатов).
/// Возвращает количество записанных. Существующая — дополняется.
pub fn create_database(dbs_dir: &Path, name: &str, usernames: &[String]) -> std::io::Result<usize> {
    let path = UserDatabase::path_for(dbs_dir, &sanitize_name(name));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    UserDatabase::append_unique(&path, usernames)
}

/// Убрать перечисленных людей из базы. Возвращает сколько реально убрано.
pub fn remove_from_database(
    dbs_dir: &Path,
    name: &str,
    usernames: &[String],
) -> std::io::Result<usize> {
    let path = UserDatabase::path_for(dbs_dir, &sanitize_name(name));
    if !path.exists() {
        return Ok(0);
    }
    let remove: HashSet<String> = usernames
        .iter()
        .map(|u| u.trim().trim_start_matches('@').to_lowercase())
        .filter(|u| !u.is_empty())
        .collect();
    let current = UserDatabase::read(&path);
    let mut kept: Vec<String> = Vec::new();
    let mut removed = 0usize;
    for u in &current {
        if remove.contains(u) {
            removed += 1;
        } else {
            kept.push(u.clone());
        }
    }
    if removed > 0 {
        let tmp = path.with_extension("txt.tmp");
        std::fs::write(&tmp, format!("{}\n", kept.join("\n")))?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(removed)
}
