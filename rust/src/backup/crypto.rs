use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{OsRng, rand_core::RngCore as _};
use chacha20poly1305::{AeadCore as _, KeyInit as _, XChaCha20Poly1305, XNonce, aead::Aead as _};
use zeroize::Zeroizing;

use cove_util::result_ext::ResultExt as _;

use super::error::BackupError;

const MAGIC: &[u8; 4] = b"COVB";
const FORMAT_VERSION: u8 = 4;

const HEADER_SIZE: usize = 49;
const MAGIC_SIZE: usize = 4;
const VERSION_SIZE: usize = 1;
const SALT_SIZE: usize = 16;
const NONCE_SIZE: usize = 24;
const PAYLOAD_LEN_SIZE: usize = 4;

const MIN_PASSWORD_LEN: usize = 20;

// matches Signal's Argon2id parameters — safe on all 2GB+ Android devices
// do not change, existing backups depend on these values
const ARGON2_M_COST: u32 = 32_768; // 32 MiB
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 1;

/// Strip all whitespace from a password and validate minimum length
pub fn clean_password(raw: &str) -> Result<Zeroizing<String>, BackupError> {
    let cleaned: String = raw.chars().filter(|c| !c.is_whitespace()).collect();

    if cleaned.chars().count() < MIN_PASSWORD_LEN {
        return Err(BackupError::PasswordTooShort);
    }

    Ok(Zeroizing::new(cleaned))
}

/// Derive a 32-byte encryption key from a password and salt using Argon2id
///
/// Parameters are pinned explicitly so old backups remain decryptable even if
/// the argon2 crate changes its defaults in a future version
fn derive_key(password: &str, salt: &[u8; SALT_SIZE]) -> Result<Zeroizing<[u8; 32]>, BackupError> {
    let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
        .map_err(|e| BackupError::Encryption(format!("invalid argon2 params: {e}")))?;

    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| BackupError::Encryption(format!("key derivation failed: {e}")))?;

    Ok(key)
}

struct ParsedHeader<'a> {
    salt: [u8; SALT_SIZE],
    nonce: &'a XNonce,
    ciphertext: &'a [u8],
}

fn parse_header(data: &[u8]) -> Result<ParsedHeader<'_>, BackupError> {
    if data.len() < HEADER_SIZE {
        return Err(BackupError::Truncated);
    }

    if &data[..MAGIC_SIZE] != MAGIC {
        return Err(BackupError::InvalidFormat);
    }

    let version = data[MAGIC_SIZE];
    if version != FORMAT_VERSION {
        return Err(BackupError::UnsupportedVersion(version as u32));
    }

    let salt: [u8; SALT_SIZE] = data
        [MAGIC_SIZE + VERSION_SIZE..MAGIC_SIZE + VERSION_SIZE + SALT_SIZE]
        .try_into()
        .map_err(|_| BackupError::Truncated)?;

    let nonce_start = MAGIC_SIZE + VERSION_SIZE + SALT_SIZE;
    let nonce = XNonce::from_slice(&data[nonce_start..nonce_start + NONCE_SIZE]);

    let payload_len_start = nonce_start + NONCE_SIZE;
    let payload_len_bytes: [u8; 4] = data[payload_len_start..payload_len_start + PAYLOAD_LEN_SIZE]
        .try_into()
        .map_err(|_| BackupError::Truncated)?;
    let payload_len = u32::from_le_bytes(payload_len_bytes) as usize;

    let payload_start = HEADER_SIZE;
    let payload_end = payload_start.checked_add(payload_len).ok_or(BackupError::Truncated)?;
    if data.len() != payload_end {
        return Err(if data.len() < payload_end {
            BackupError::Truncated
        } else {
            BackupError::InvalidFormat
        });
    }

    let ciphertext = &data[payload_start..payload_end];

    Ok(ParsedHeader { salt, nonce, ciphertext })
}

/// Encrypt data and assemble the .covb file bytes
pub fn encrypt(plaintext: &[u8], password: &str) -> Result<Vec<u8>, BackupError> {
    let mut salt = [0u8; SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    let key = derive_key(password, &salt)?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);

    let ciphertext =
        cipher.encrypt(&nonce, plaintext).map_err(|e| BackupError::Encryption(e.to_string()))?;

    let payload_len: u32 = ciphertext
        .len()
        .try_into()
        .map_err(|_| BackupError::Encryption("payload too large".into()))?;

    let mut output = Vec::with_capacity(HEADER_SIZE + ciphertext.len());
    output.extend_from_slice(MAGIC);
    output.push(FORMAT_VERSION);
    output.extend_from_slice(&salt);
    output.extend_from_slice(nonce.as_slice());
    output.extend_from_slice(&payload_len.to_le_bytes());
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// Parse a .covb file and decrypt its payload
pub fn decrypt(data: &[u8], password: &str) -> Result<Zeroizing<Vec<u8>>, BackupError> {
    let header = parse_header(data)?;

    let key = derive_key(password, &header.salt)?;
    let cipher = XChaCha20Poly1305::new((&*key).into());

    cipher
        .decrypt(header.nonce, header.ciphertext)
        .map(Zeroizing::new)
        .map_err(|_| BackupError::DecryptionFailed)
}

/// Validate the file header and payload length without decrypting
pub fn validate_header(data: &[u8]) -> Result<(), BackupError> {
    parse_header(data).map(|_| ())
}

/// Compress data using zstd
pub fn compress(data: &[u8]) -> Result<Vec<u8>, BackupError> {
    Ok(ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest))
}

/// 10 MB — wallet backups are realistically a few MB at most
const MAX_DECOMPRESSED_SIZE: usize = 10 * 1024 * 1024;

/// Decompress zstd-compressed data
pub fn decompress(data: &[u8]) -> Result<Zeroizing<Vec<u8>>, BackupError> {
    use std::io::Read as _;
    let decoder =
        ruzstd::decoding::StreamingDecoder::new(data).map_err_str(BackupError::Decompression)?;

    let mut output = Zeroizing::new(Vec::new());
    let mut reader = decoder.take(MAX_DECOMPRESSED_SIZE as u64 + 1);
    reader.read_to_end(&mut output).map_err_str(BackupError::Decompression)?;

    if output.len() > MAX_DECOMPRESSED_SIZE {
        return Err(BackupError::Decompression(format!(
            "decompressed size exceeds {MAX_DECOMPRESSED_SIZE} byte limit"
        )));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argon2_pinned_constants_unchanged() {
        assert_eq!(ARGON2_M_COST, 32_768);
        assert_eq!(ARGON2_T_COST, 3);
        assert_eq!(ARGON2_P_COST, 1);
    }

    #[test]
    fn clean_password_strips_whitespace() {
        let cleaned = clean_password("abandon ability   able about\tabove absent absorb").unwrap();
        assert_eq!(&*cleaned, "abandonabilityableaboutaboveabsentabsorb");
    }

    #[test]
    fn clean_password_rejects_short() {
        let result = clean_password("tooshort");
        assert!(matches!(result, Err(BackupError::PasswordTooShort)));
    }

    #[test]
    fn clean_password_accepts_20_chars() {
        let password = "a".repeat(20);
        assert!(clean_password(&password).is_ok());
    }

    #[test]
    fn clean_password_rejects_19_chars() {
        let password = "a".repeat(19);
        assert!(matches!(clean_password(&password), Err(BackupError::PasswordTooShort)));
    }

    #[test]
    fn clean_password_accepts_with_whitespace_if_cleaned_long_enough() {
        let password = "a b c d e f g h i j k l m n o p q r s t";
        let cleaned = clean_password(password).unwrap();
        assert_eq!(cleaned.len(), 20);
    }

    #[test]
    fn round_trip_encrypt_decrypt() {
        let plaintext = b"hello world, this is a backup payload";
        let password = "abandonabilityableaboutaboveabsentabsorb";

        let encrypted = encrypt(plaintext, password).unwrap();
        let decrypted = decrypt(&encrypted, password).unwrap();

        assert_eq!(&*decrypted, plaintext);
    }

    #[test]
    fn wrong_password_fails() {
        let plaintext = b"secret data";
        let password = "abandonabilityableaboutaboveabsentabsorb";
        let wrong = "wrongpasswordwrongpasswordwrongpassword1";

        let encrypted = encrypt(plaintext, password).unwrap();
        let result = decrypt(&encrypted, wrong);

        assert!(matches!(result, Err(BackupError::DecryptionFailed)));
    }

    #[test]
    fn invalid_magic_fails() {
        let mut data = vec![0u8; 100];
        data[..4].copy_from_slice(b"NOPE");

        assert!(matches!(validate_header(&data), Err(BackupError::InvalidFormat)));
    }

    #[test]
    fn truncated_file_fails() {
        let data = vec![0u8; 10];
        assert!(matches!(validate_header(&data), Err(BackupError::Truncated)));
    }

    #[test]
    fn unsupported_version_fails() {
        let mut data = vec![0u8; 100];
        data[..4].copy_from_slice(MAGIC);
        data[4] = 99;

        assert!(matches!(validate_header(&data), Err(BackupError::UnsupportedVersion(99))));
    }

    #[test]
    fn version_1_rejected() {
        let mut data = vec![0u8; 100];
        data[..4].copy_from_slice(MAGIC);
        data[4] = 1;

        assert!(matches!(validate_header(&data), Err(BackupError::UnsupportedVersion(1))));
    }

    #[test]
    fn version_2_rejected() {
        let mut data = vec![0u8; 100];
        data[..4].copy_from_slice(MAGIC);
        data[4] = 2;

        assert!(matches!(validate_header(&data), Err(BackupError::UnsupportedVersion(2))));
    }

    #[test]
    fn version_3_rejected() {
        let mut data = vec![0u8; 100];
        data[..4].copy_from_slice(MAGIC);
        data[4] = 3;

        assert!(matches!(validate_header(&data), Err(BackupError::UnsupportedVersion(3))));
    }

    #[test]
    fn header_size_matches_components() {
        // hardcoded so the test breaks if any component constant changes
        assert_eq!(HEADER_SIZE, 4 + 1 + 16 + 24 + 4);
    }

    #[test]
    fn compress_decompress_round_trip() {
        let original = b"hello world hello world hello world";
        let compressed = compress(original).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(&*decompressed, original);
    }
}
