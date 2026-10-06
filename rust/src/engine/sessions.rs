//! Кодек telethon/gramjs StringSession.
//!
//! Формат (v1): символ '1' + base64([dc_id u8][len адреса i16 BE][адрес]
//! [port i16 BE][auth_key 256 байт]). Вариант Telethon (base64-длина == 352):
//! [dc_id u8][ipv4 4 байта][port i16 BE][auth_key].

use base64::Engine;

/// Разобранная StringSession.
#[derive(Debug, Clone)]
pub struct StringSessionData {
    pub dc_id: i32,
    pub ip: String,
    pub port: i32,
    pub auth_key: [u8; 256],
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("не StringSession (нет версии '1')")]
    NotStringSession,
    #[error("base64: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("неожиданный конец данных")]
    TooShort,
    #[error("auth key не 256 байт: {0}")]
    BadKeyLen(usize),
}

/// Декодировать base64 StringSession с учётом того, что Telethon кодирует
/// через `base64.urlsafe_b64encode` (алфавит `-`/`_`), а gramjs — стандартным.
/// Пробуем urlsafe, затем standard, затем варианты без паддинга.
fn decode_base64(b64: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    URL_SAFE
        .decode(b64)
        .or_else(|_| STANDARD.decode(b64))
        .or_else(|_| URL_SAFE_NO_PAD.decode(b64))
        .or_else(|_| STANDARD_NO_PAD.decode(b64))
}

impl StringSessionData {
    /// Декодировать gramjs/telethon StringSession.
    pub fn decode(session: &str) -> Result<Self, SessionError> {
        let s = session.trim();
        if !s.starts_with('1') || s.len() < 10 {
            return Err(SessionError::NotStringSession);
        }
        let b64 = &s[1..];
        let bytes = decode_base64(b64)?;
        // Тело: либо telethon (dc + ipv4 + port + key = 263 байта, IPv4 без
        // длины адреса), либо gramjs (длина адреса перед адресом).
        // Считаем по длине ДЕКОДИРОВАННЫХ байт: urlsafe-строка без паддинга
        // короче 352 символов, но тело всё равно 263 байта.
        let telethon = bytes.len() == 263;
        let mut pos = 0usize;
        let need = |n: usize, pos: usize, len: usize| -> Result<(), SessionError> {
            if pos + n > len {
                Err(SessionError::TooShort)
            } else {
                Ok(())
            }
        };
        need(1, pos, bytes.len())?;
        let dc_id = bytes[pos] as i32;
        pos += 1;

        let ip: String;
        if telethon {
            need(4, pos, bytes.len())?;
            ip = format!("{}.{}.{}.{}", bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]);
            pos += 4;
        } else {
            need(2, pos, bytes.len())?;
            let addr_len = i16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as i32;
            pos += 2;
            if addr_len > 100 {
                // IPv6: грамjs отматывает offset на 2 (16 байт начинаются с места
                // ПОЛЯ ДЛИНЫ, а не после него) — StringSession.js:51 reader.offset -= 2
                pos -= 2;
                need(16, pos, bytes.len())?;
                let mut s = String::new();
                for (i, b) in bytes[pos..pos + 16].iter().enumerate() {
                    if i % 2 == 0 && i > 0 {
                        s.push(':');
                    }
                    s.push_str(&format!("{b:02x}"));
                }
                ip = s;
                pos += 16;
            } else {
                need(addr_len as usize, pos, bytes.len())?;
                ip = String::from_utf8_lossy(&bytes[pos..pos + addr_len as usize]).into_owned();
                pos += addr_len as usize;
            }
        }

        need(2, pos, bytes.len())?;
        let port = i16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as i32;
        pos += 2;

        let key_len = bytes.len() - pos;
        if key_len < 256 {
            return Err(SessionError::BadKeyLen(key_len));
        }
        let mut auth_key = [0u8; 256];
        auth_key.copy_from_slice(&bytes[pos..pos + 256]);

        Ok(StringSessionData { dc_id, ip, port, auth_key })
    }

    /// Кодировать в gramjs StringSession (адрес строкой).
    pub fn encode(&self) -> String {
        let mut body = Vec::with_capacity(1 + 2 + self.ip.len() + 2 + 256);
        body.push(self.dc_id as u8);
        let ip_bytes = self.ip.as_bytes();
        body.extend_from_slice(&(ip_bytes.len() as i16).to_be_bytes());
        body.extend_from_slice(ip_bytes);
        body.extend_from_slice(&(self.port as i16).to_be_bytes());
        body.extend_from_slice(&self.auth_key);
        let b64 = base64::engine::general_purpose::STANDARD.encode(body);
        format!("1{b64}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StringSessionData {
        let mut auth_key = [0u8; 256];
        for (i, v) in auth_key.iter_mut().enumerate() {
            *v = (i % 251) as u8;
        }
        StringSessionData {
            dc_id: 2,
            ip: "149.154.167.50".to_string(),
            port: 443,
            auth_key,
        }
    }

    #[test]
    fn roundtrip() {
        let s = sample();
        let encoded = s.encode();
        assert!(encoded.starts_with('1'));
        let decoded = StringSessionData::decode(&encoded).unwrap();
        assert_eq!(decoded.dc_id, 2);
        assert_eq!(decoded.ip, "149.154.167.50");
        assert_eq!(decoded.port, 443);
        assert_eq!(decoded.auth_key, s.auth_key);
    }

    #[test]
    fn rejects_garbage() {
        assert!(StringSessionData::decode("").is_err());
        assert!(StringSessionData::decode("QmFkU2Vzc2lvbg==").is_err());
    }

    #[test]
    fn decodes_urlsafe_telethon_session() {
        // Telethon кодирует StringSession через base64.urlsafe_b64encode.
        // Собираем валидную telethon-строку (dc + ipv4 + port + auth_key),
        // гарантируя наличие urlsafe-символов '-'/'_', которые STANDARD
        // декодер не принимает (регрессия B.1.1).
        let mut auth_key = [0u8; 256];
        auth_key[0] = 0xff; // 111111 -> '_' в urlsafe-алфавите
        let mut body = Vec::with_capacity(1 + 4 + 2 + 256);
        body.push(2u8); // dc_id
        body.extend_from_slice(&[149, 154, 167, 50]); // ipv4
        body.extend_from_slice(&443i16.to_be_bytes()); // port
        body.extend_from_slice(&auth_key);
        let encoded = format!(
            "1{}",
            base64::engine::general_purpose::URL_SAFE.encode(&body)
        );
        assert!(
            encoded.contains('_') || encoded.contains('-'),
            "пример должен содержать urlsafe-символы: {encoded}"
        );
        assert!(
            base64::engine::general_purpose::STANDARD
                .decode(&encoded[1..])
                .is_err(),
            "STANDARD-декодер не должен принимать urlsafe-строку"
        );

        let decoded = StringSessionData::decode(&encoded).unwrap();
        assert_eq!(decoded.dc_id, 2);
        assert_eq!(decoded.ip, "149.154.167.50");
        assert_eq!(decoded.port, 443);
        assert_eq!(decoded.auth_key, auth_key);
    }

    #[test]
    fn decodes_urlsafe_without_padding() {
        let mut auth_key = [0u8; 256];
        auth_key[0] = 0xfb; // 111110 -> '-' в urlsafe-алфавите
        let mut body = Vec::with_capacity(1 + 4 + 2 + 256);
        body.push(2u8);
        body.extend_from_slice(&[149, 154, 167, 50]);
        body.extend_from_slice(&443i16.to_be_bytes());
        body.extend_from_slice(&auth_key);
        let encoded = format!(
            "1{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&body)
        );
        assert!(!encoded.contains('='), "паддинг должен отсутствовать");
        let decoded = StringSessionData::decode(&encoded).unwrap();
        assert_eq!(decoded.dc_id, 2);
        assert_eq!(decoded.ip, "149.154.167.50");
        assert_eq!(decoded.port, 443);
        assert_eq!(decoded.auth_key, auth_key);
    }
}
