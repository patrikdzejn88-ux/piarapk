//! Парсер tdata (Telegram Desktop) → (user_id, dc, auth_key).
//! Порт TS-конвертера из tg-piar (gist painor / MadelineProto, AGPL-3.0).
//!
//! Ограничения порта: 1 аккаунт, пустой локальный passcode, magic 75.

use std::path::Path;

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use md5::Digest as _;
use sha1::Digest as _;

type Aes256IgeEnc = ige::Encryptor<aes::Aes256>;
type Aes256IgeDec = ige::Decryptor<aes::Aes256>;

#[derive(Debug, thiserror::Error)]
pub enum TdataError {
    #[error("файл не найден: {0}")]
    NotFound(String),
    #[error("неверный magic (не tdata-файл): {0}")]
    WrongMagic(String),
    #[error("md5-контрольная сумма не сошлась (файл повреждён)")]
    WrongMd5,
    #[error("длина salt != 32 — это не tdata")]
    BadSalt,
    #[error("msg_key mismatch (passcode не пуст или формат не поддержан)")]
    MsgKeyMismatch,
    #[error("поддерживается только 1 аккаунт в tdata (найдено {0})")]
    MultipleAccounts(u32),
    #[error("неподдерживаемая magic-версия: {0}")]
    UnsupportedMagic(u32),
    #[error("auth key для main DC не найден")]
    NoMainDcKey,
    #[error("битый формат данных: {0}")]
    Malformed(&'static str),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Результат разбора tdata.
pub struct TdataInfo {
    pub user_id: u32,
    pub main_dc: i32,
    pub auth_key: [u8; 256],
    pub ip: String,
    pub port: i32,
}

/// md5-хеш имени файла в стиле tdesktop (hex-пары переставлены местами, uppercase).
pub fn tdesktop_md5(data: &str) -> String {
    let mut h = md5::Md5::new();
    h.update(data.as_bytes());
    let digest = hex::encode(h.finalize());
    let mut out = String::with_capacity(digest.len());
    let bytes: Vec<char> = digest.chars().collect();
    let mut i = 0;
    while i + 1 < bytes.len() {
        out.push(bytes[i + 1]);
        out.push(bytes[i]);
        i += 2;
    }
    out.to_uppercase()
}

/// Статические IP DC Telegram (как в старом конвертере).
pub fn dc_ip(dc: i32) -> Option<&'static str> {
    Some(match dc {
        1 => "149.154.175.55",
        2 => "149.154.167.50",
        3 => "149.154.175.100",
        4 => "149.154.167.91",
        5 => "91.108.56.170",
        _ => return None,
    })
}

/// Потоковый читатель tdata-данных.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn read(&mut self, n: usize) -> Result<&'a [u8], TdataError> {
        if self.pos + n > self.buf.len() {
            return Err(TdataError::Malformed("неожиданный конец данных"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// u32/i32 big-endian (в tdesktop — QDataStream: байты читаются «reverse + LE»).
    fn read_be_u32(&mut self) -> Result<u32, TdataError> {
        let b = self.read(4)?;
        let mut r = [0u8; 4];
        r.copy_from_slice(b);
        r.reverse();
        Ok(u32::from_le_bytes(r))
    }

    /// length-prefixed буфер (длина BE i32).
    fn read_buffer(&mut self) -> Result<&'a [u8], TdataError> {
        let b = self.read(4)?;
        let mut r = [0u8; 4];
        r.copy_from_slice(b);
        r.reverse();
        let length = i32::from_le_bytes(r);
        if length <= 0 {
            return Ok(&[]);
        }
        let length = length as usize;
        if self.pos + length > self.buf.len() {
            return Err(TdataError::Malformed("буфер длиннее остатка"));
        }
        let s = &self.buf[self.pos..self.pos + length];
        self.pos += length;
        Ok(s)
    }

    fn read_rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos.min(self.buf.len())..];
        self.pos = self.buf.len();
        s
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }
}

/// Открыть TDF$-файл: проверить magic, версию, md5; вернуть payload.
fn open_tdf(path: &Path) -> Result<Vec<u8>, TdataError> {
    if !path.exists() {
        return Err(TdataError::NotFound(path.display().to_string()));
    }
    let raw = std::fs::read(path)?;
    if raw.len() < 24 {
        return Err(TdataError::Malformed("файл слишком короткий"));
    }
    if &raw[0..4] != b"TDF$" {
        return Err(TdataError::WrongMagic(path.display().to_string()));
    }
    let version_bytes = [raw[4], raw[5], raw[6], raw[7]];
    let _version = i32::from_le_bytes(version_bytes);
    let body_with_md5 = &raw[8..];
    if body_with_md5.len() < 16 {
        return Err(TdataError::Malformed("нет md5-хвоста"));
    }
    let (data, md5_tail) = body_with_md5.split_at(body_with_md5.len() - 16);

    let mut to_compare = Vec::with_capacity(data.len() + 4 + 4 + 4);
    to_compare.extend_from_slice(data);
    to_compare.extend_from_slice(&(data.len() as i32).to_le_bytes());
    to_compare.extend_from_slice(&version_bytes);
    to_compare.extend_from_slice(b"TDF$");
    let mut h = md5::Md5::new();
    h.update(&to_compare);
    let hash = h.finalize();
    if hash.as_slice() != md5_tail {
        return Err(TdataError::WrongMd5);
    }
    Ok(data.to_vec())
}

/// Расчёт AES ключей (как tdesktop; client=false → x=8).
fn calc_key(auth_key: &[u8], msg_key: &[u8]) -> ([u8; 32], [u8; 32]) {
    let x = 8usize;
    let slice = |from: usize, len: usize| &auth_key[from..from + len];

    let mut sha1_a_h = sha1::Sha1::new();
    sha1_a_h.update(msg_key);
    sha1_a_h.update(slice(x, 32));
    let sha1_a = sha1_a_h.finalize();

    let mut sha1_b_h = sha1::Sha1::new();
    sha1_b_h.update(slice(32 + x, 16));
    sha1_b_h.update(msg_key);
    sha1_b_h.update(slice(48 + x, 16));
    let sha1_b = sha1_b_h.finalize();

    let mut sha1_c_h = sha1::Sha1::new();
    sha1_c_h.update(slice(64 + x, 32));
    sha1_c_h.update(msg_key);
    let sha1_c = sha1_c_h.finalize();

    let mut sha1_d_h = sha1::Sha1::new();
    sha1_d_h.update(msg_key);
    sha1_d_h.update(slice(96 + x, 32));
    let sha1_d = sha1_d_h.finalize();

    let mut aes_key = [0u8; 32];
    let mut aes_iv = [0u8; 32];

    aes_key[0..8].copy_from_slice(&sha1_a[0..8]);
    aes_key[8..20].copy_from_slice(&sha1_b[8..20]);
    aes_key[20..32].copy_from_slice(&sha1_c[4..16]);

    aes_iv[0..12].copy_from_slice(&sha1_a[8..20]);
    aes_iv[12..20].copy_from_slice(&sha1_b[0..8]);
    aes_iv[20..24].copy_from_slice(&sha1_c[16..20]);
    aes_iv[24..32].copy_from_slice(&sha1_d[0..8]);

    (aes_key, aes_iv)
}

/// Дешифровка AES-IGE с проверкой msg_key.
fn decrypt_ige(data: &[u8], auth_key: &[u8]) -> Result<Vec<u8>, TdataError> {
    if data.len() < 16 || (data.len() - 16) % 16 != 0 || auth_key.len() < 136 {
        return Err(TdataError::Malformed("кривые длины для IGE"));
    }
    let (msg_key, encrypted) = data.split_at(16);
    let (aes_key, aes_iv) = calc_key(auth_key, msg_key);
    let mut buf = encrypted.to_vec();
    let mut cipher = Aes256IgeDec::new_from_slices(&aes_key, &aes_iv)
        .map_err(|_| TdataError::Malformed("bad key/iv"))?;
    for chunk in buf.chunks_exact_mut(16) {
        let block = GenericArray::from_mut_slice(chunk);
        cipher.decrypt_block_mut(block);
    }

    let mut sha1_h = sha1::Sha1::new();
    sha1_h.update(&buf);
    let digest = sha1_h.finalize();
    if digest[0..16] != msg_key[0..16] {
        return Err(TdataError::MsgKeyMismatch);
    }
    Ok(buf)
}

/// Локальный ключ для ПУСТОГО passcode: SHA512(salt+""+salt) → PBKDF2-SHA512(1, 256).
fn local_key(salt: &[u8]) -> [u8; 256] {
    let mut h = sha2::Sha512::new();
    sha2::Digest::update(&mut h, salt);
    sha2::Digest::update(&mut h, b"");
    sha2::Digest::update(&mut h, salt);
    let hash = h.finalize();
    let mut out = [0u8; 256];
    pbkdf2::pbkdf2_hmac::<sha2::Sha512>(&hash, salt, 1, &mut out);
    out
}

/// Разобрать папку tdata (с key_data*, map*, <md5>/...) → TdataInfo.
pub fn parse_tdata(tdata_dir: &Path) -> Result<TdataInfo, TdataError> {
    let key_data_prefix = tdata_dir.join("key_data");
    let payload = open_tdf_first(&key_data_prefix)?;

    let mut r = Reader::new(&payload);
    // salt
    let salt_b = r.read_buffer()?;
    if salt_b.len() != 32 {
        return Err(TdataError::BadSalt);
    }
    let salt: Vec<u8> = salt_b.to_vec();
    let encrypted_key = r.read_buffer()?.to_vec();
    let encrypted_info = r.read_buffer()?.to_vec();

    let pass_key = local_key(&salt);

    // key = readBuffer(decrypt(encrypted_key, pass_key))
    let key_plain = decrypt_ige(&encrypted_key, &pass_key)?;
    let mut kr = Reader::new(&key_plain);
    let key = kr.read_buffer()?.to_vec();

    // info = readBuffer(decrypt(encrypted_info, key))
    let info_plain = decrypt_ige(&encrypted_info, &key)?;
    let mut ir = Reader::new(&info_plain);
    let info = ir.read_buffer()?;
    if info.len() < 4 {
        return Err(TdataError::Malformed("info слишком короткий"));
    }
    let count = u32::from_be_bytes([info[0], info[1], info[2], info[3]]);
    if count != 1 {
        return Err(TdataError::MultipleAccounts(count));
    }

    // пользовательельская папка: tdata_dir/<md5("data")[0..16]>/
    let part_one_md5 = &tdesktop_md5("data")[0..16];
    let user_base = tdata_dir.join(part_one_md5);

    let main_payload = open_tdf_first(&user_base)?;
    let mut br = Reader::new(&main_payload);
    let inner = br.read_buffer()?.to_vec();
    let mut decrypted = decrypt_ige(&inner, &key)?;

    // длина-заголовок (LE i32, одно чтение 4 байта) — только валидация
    {
        let mut mr = Reader::new(&decrypted);
        let b = mr.read(4)?;
        let len = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        if len as usize > decrypted.len() || len < 4 {
            return Err(TdataError::Malformed("wrong length в зашифрованном файле"));
        }
    }
    // отбрасываем первые 4 байта длины
    decrypted.drain(0..4);

    let mut main = Reader::new(&decrypted);
    let magic = main.read_be_u32()?;
    if magic != 75 {
        return Err(TdataError::UnsupportedMagic(magic));
    }
    let final_buf = main.read_buffer()?.to_vec();
    let mut fr = Reader::new(&final_buf);
    fr.read(12)?; // пропустить 12 байт
    let user_id = fr.read_be_u32()?;
    let main_dc = fr.read_be_u32()? as i32;
    let entries = fr.read_be_u32()?;

    for _ in 0..entries {
        let dc = fr.read_be_u32()? as i32;
        let key_bytes = fr.read(256)?;
        if dc == main_dc {
            let mut auth_key = [0u8; 256];
            auth_key.copy_from_slice(key_bytes);
            let ip = dc_ip(main_dc)
                .ok_or(TdataError::Malformed("неизвестный DC"))?
                .to_string();
            return Ok(TdataInfo {
                user_id,
                main_dc,
                auth_key,
                ip,
                port: 443,
            });
        }
    }
    Err(TdataError::NoMainDcKey)
}

/// Открыть первый ВАЛИДНЫЙ TDF$-файл из вариантов name+0/1/s (эталон
/// перебирает все, а не падает на первом битом).
fn open_tdf_first(prefix: &Path) -> Result<Vec<u8>, TdataError> {
    let mut last_err = None;
    for suffix in ["0", "1", "s"] {
        let p = Path::new(&format!("{}{}", prefix.display(), suffix));
        if p.exists() {
            match open_tdf(p) {
                Ok(payload) => return Ok(payload),
                Err(e) => last_err = Some(e),
            }
        }
    }
    Err(last_err.unwrap_or_else(|| TdataError::NotFound(format!("{}0/1/s", prefix.display()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_name_style() {
        // структура: hex-пары переставлены + uppercase; длина = 32
        let h = tdesktop_md5("data");
        assert_eq!(h.len(), 32);
        assert!(h.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        // пара-перестановка проверяется обратным преобразованием
        let digest = hex::encode(md5::Md5::digest(b"data"));
        let mut expected = String::new();
        let chars: Vec<char> = digest.chars().collect();
        let mut i = 0;
        while i + 1 < chars.len() {
            expected.push(chars[i + 1]);
            expected.push(chars[i]);
            i += 2;
        }
        assert_eq!(h, expected.to_uppercase());
    }

    #[test]
    fn be_reader() {
        // BE-число 0x00000030 (48) в файле: [0,0,0,0x30] → reverse+LE = 48
        let mut r = Reader::new(&[0, 0, 0, 0x30, 1, 2, 3, 4]);
        assert_eq!(r.read_be_u32().unwrap(), 48);
        assert_eq!(r.read(4).unwrap(), &[1, 2, 3, 4]);
    }

    #[test]
    fn length_prefixed_buffer() {
        // длина 4 (BE: [0,0,0,4]) + 4 байта данных
        let data = [0, 0, 0, 4, 9, 9, 9, 9];
        let mut r = Reader::new(&data);
        let b = r.read_buffer().unwrap();
        assert_eq!(b, &[9, 9, 9, 9]);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn local_key_shape() {
        let salt = [7u8; 32];
        let k = local_key(&salt);
        assert_eq!(k.len(), 256);
    }

    #[test]
    fn ige_roundtrip_local() {
        // самопроверка IGE: encrypt тем же ключом расшифровывается обратно
        let key = [3u8; 32];
        let iv = [5u8; 32];
        let plain = vec![42u8; 32];
        let mut enc = plain.clone();
        let mut c = Aes256IgeEnc::new_from_slices(&key, &iv).unwrap();
        for chunk in enc.chunks_exact_mut(16) {
            let block = GenericArray::from_mut_slice(chunk);
            c.encrypt_block_mut(block);
        }
        let mut d = Aes256IgeDec::new_from_slices(&key, &iv).unwrap();
        for chunk in enc.chunks_exact_mut(16) {
            let block = GenericArray::from_mut_slice(chunk);
            d.decrypt_block_mut(block);
        }
        assert_eq!(enc, plain);
    }
}
