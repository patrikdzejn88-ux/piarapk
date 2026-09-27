//! Чаты: реестр (chats.json), разрешение @username, постинг, read-only канал.

use grammers_client::peer::Peer;
use grammers_session::types::{PeerAuth, PeerId, PeerRef};

use super::state::AppState;
use super::store::{self, ChatRecord};

/// Нормализовать ссылку на чат → username без @/t.me/.
pub fn normalize_chat_link(link: &str) -> String {
    let mut s = link.trim().to_string();
    if let Some(rest) = s.strip_prefix("https://t.me/") {
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix("http://t.me/") {
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix("t.me/") {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_prefix('@') {
        s = rest.to_string();
    }
    s.split('/').next().unwrap_or_default().to_string()
}

/// Разрешить @username в Peer (любым подключённым аккаунтом).
pub async fn resolve_chat(state: &AppState, link: &str) -> anyhow::Result<Peer> {
    let username = normalize_chat_link(link);
    if username.is_empty() {
        return Err(anyhow::anyhow!("пустая ссылка на чат"));
    }
    let Some(client) = super::connect::connected_client(state, "piar") else {
        return Err(anyhow::anyhow!("нет подключённых аккаунтов — подключите хотя бы один"));
    };
    // resolve_username возвращает Option<Peer>
    let peer: Peer = client
        .resolve_username(&username)
        .await
        .map_err(|e| anyhow::anyhow!("ошибка поиска: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("чат @{username} не найден"))?;
    Ok(peer)
}

/// Добавить чат в реестр (с подсчётом участников).
pub async fn add_chat(state: &AppState, link: &str) -> anyhow::Result<serde_json::Value> {
    let peer = resolve_chat(state, link).await?;
    let peer_ref = peer
        .to_ref()
        .await
        .map_err(|e| anyhow::anyhow!("to_ref: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("пустой PeerRef"))?;
    let dialog_id = peer_ref
        .id
        .bot_api_dialog_id()
        .ok_or_else(|| anyhow::anyhow!("unsupported peer kind"))?;
    let access_hash = peer_ref.auth.hash();
    let title = peer.name().unwrap_or("без названия").to_string();

    // количество участников (для UI)
    let mut members: i64 = 0;
    if let Some(client) = super::connect::connected_client(state, "piar") {
        let mut it = client.iter_participants(peer_ref);
        members = it.total().await.unwrap_or(0) as i64;
    }

    let record = ChatRecord {
        dialog_id,
        access_hash,
        title,
        members,
        added_at: now_secs(),
    };
    let mut chats = state.chats.lock();
    chats.chats.retain(|c| c.dialog_id != dialog_id);
    chats.chats.push(record.clone());
    let dir = state.data_dir.clone();
    store::save_chats(&dir, &chats)?;
    drop(chats);
    Ok(serde_json::json!({
        "id": record.dialog_id,
        "title": record.title,
        "access_hash": record.access_hash,
    }))
}

/// Восстановить PeerRef из записи чата.
pub fn chat_peer_ref(record: &ChatRecord) -> anyhow::Result<PeerRef> {
    let id = PeerId::from_bot_api_dialog_id(record.dialog_id)
        .ok_or_else(|| anyhow::anyhow!("неизвестный dialog_id: {}", record.dialog_id))?;
    Ok(PeerRef {
        id,
        auth: PeerAuth::from_hash(record.access_hash),
    })
}

/// Найти запись чата по dialog_id.
pub fn find_chat(state: &AppState, chat_id: i64) -> Option<ChatRecord> {
    state.chats.lock().chats.iter().find(|c| c.dialog_id == chat_id).cloned()
}

/// Список чатов → JSON.
pub fn chats_json(state: &AppState) -> serde_json::Value {
    let chats = state.chats.lock();
    let list: Vec<serde_json::Value> = chats
        .chats
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.dialog_id,
                "title": c.title,
                "members": c.members,
            })
        })
        .collect();
    serde_json::Value::Array(list)
}

/// Отправить сообщение в чат (первым подключённым аккаунтом).
pub async fn post_message(
    state: &AppState,
    chat_id: i64,
    text: &str,
) -> anyhow::Result<serde_json::Value> {
    let record = find_chat(state, chat_id)
        .ok_or_else(|| anyhow::anyhow!("чат не найден: {chat_id}"))?;
    let peer_ref = chat_peer_ref(&record)?;
    let Some(client) = super::connect::connected_client(state, "piar") else {
        return Err(anyhow::anyhow!("нет подключённых аккаунтов"));
    };
    client
        .send_message(peer_ref, text)
        .await
        .map_err(|e| anyhow::anyhow!("не отправилось: {e}"))?;
    Ok(serde_json::json!({ "sent": true }))
}

/// Создать broadcast-канал (пишут только админы — read-only для подписчиков).
pub async fn create_readonly_channel(
    state: &AppState,
    title: &str,
    about: &str,
) -> anyhow::Result<serde_json::Value> {
    let Some(client) = super::connect::connected_client(state, "piar") else {
        return Err(anyhow::anyhow!("нет подключённых аккаунтов"));
    };
    let request = grammers_tl_types::functions::channels::CreateChannel {
        broadcast: true,
        megagroup: false,
        for_import: false,
        forum: false,
        title: title.to_string(),
        about: about.to_string(),
        geo_point: None,
        address: None,
        ttl_period: None,
    };
    let updates = client
        .invoke(&request)
        .await
        .map_err(|e| anyhow::anyhow!("createChannel: {e}"))?;

    // вытащить канал из Updates (обычно Updates::Updates { chats, .. })
    use grammers_tl_types::enums::Updates;
    let mut channel: Option<(i64, i64, String)> = None;
    if let Updates::Updates(u) = updates {
        for chat in u.chats {
            if let grammers_tl_types::enums::Chat::Channel(c) = chat {
                channel = Some((c.id, c.access_hash.unwrap_or_default(), c.title.clone()));
                break;
            }
        }
    }
    let Some((id, access_hash, title)) = channel else {
        return Err(anyhow::anyhow!("не удалось прочитать созданный канал из ответа"));
    };
    let dialog_id = PeerId::channel(id)
        .and_then(|p| p.bot_api_dialog_id())
        .ok_or_else(|| anyhow::anyhow!("channel peer id"))?;

    let record = ChatRecord {
        dialog_id,
        access_hash,
        title,
        members: 0,
        added_at: now_secs(),
    };
    {
        let mut chats = state.chats.lock();
        chats.chats.push(record.clone());
        store::save_chats(&state.data_dir, &chats)?;
    }
    Ok(serde_json::json!({
        "id": record.dialog_id,
        "title": record.title,
    }))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
