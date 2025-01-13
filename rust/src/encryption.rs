use base64::prelude::BASE64_STANDARD;
use base64::Engine as _;
use chacha20poly1305::{aead::Aead as _, AeadCore as _, ChaCha20Poly1305, KeyInit as _};
use chacha20poly1305::{Key, Nonce};
use rand::rngs::OsRng;

use macros::impl_default_for;

const SPLITTER: &str = "::";

#[derive(Debug, Clone)]
pub struct Cryptor {
    key: Key,
    nonce: Nonce,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("string is not in the correct format")]
    KeyAndNonceNotFound,

    #[error("key not in base64 format")]
    KeyInvalidFormat(base64::DecodeError),

    #[error("nonce not in base64 format")]
    NonceInvalidFormat(base64::DecodeError),

    #[error("unable to encrypt: {0}")]
    UnableToEncrypt(chacha20poly1305::Error),

    #[error("unable to decrypt: {0}")]
    UnableToDecrypt(chacha20poly1305::Error),

    #[error("base64 decode error: {0}")]
    Base64Decode(base64::DecodeError),

    #[error("invalid utf8 string")]
    InvalidUtf8(std::string::FromUtf8Error),
}

impl_default_for!(Cryptor);
impl Cryptor {
    pub fn new() -> Self {
        let key = ChaCha20Poly1305::generate_key(&mut OsRng);
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);

        Self { key, nonce }
    }

    pub fn try_from_string(string: String) -> Result<Self, Error> {
        let (key_string, nonce_string) = string
            .split_once(SPLITTER)
            .ok_or(Error::KeyAndNonceNotFound)?;

        let key_bytes = BASE64_STANDARD
            .decode(key_string.as_bytes())
            .map_err(Error::KeyInvalidFormat)?;

        let key = Key::from_slice(&key_bytes);

        let nonce_bytes = BASE64_STANDARD
            .decode(nonce_string.as_bytes())
            .map_err(Error::NonceInvalidFormat)?;

        let nonce = Nonce::from_slice(&nonce_bytes);

        Ok(Self {
            key: *key,
            nonce: *nonce,
        })
    }

    pub fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(&self.key)
    }

    pub fn serialize_to_string(self) -> String {
        let key_bytes = self.key.as_slice();
        let key_string = BASE64_STANDARD.encode(key_bytes);

        let nonce_bytes = self.nonce.as_slice();
        let nonce_string = BASE64_STANDARD.encode(nonce_bytes);

        format!("{key_string}{SPLITTER}{nonce_string}")
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let encrypted = self
            .cipher()
            .encrypt(&self.nonce, plaintext)
            .map_err(Error::UnableToEncrypt)?;

        Ok(encrypted)
    }

    pub fn encrypt_to_string(&self, plaintext: String) -> Result<String, Error> {
        let plaintext = plaintext.as_bytes();
        let encrypted = self.encrypt(plaintext)?;
        let encrypted_string = BASE64_STANDARD.encode(&encrypted);

        Ok(encrypted_string)
    }

    pub fn decrypt_from_string(&self, ciphertext: &str) -> Result<String, Error> {
        let ciphertext_bytes = BASE64_STANDARD
            .decode(ciphertext.as_bytes())
            .map_err(Error::Base64Decode)?;

        let decrypted = self.decrypt(&ciphertext_bytes)?;

        let decrypted_string = String::from_utf8(decrypted).map_err(Error::InvalidUtf8)?;
        Ok(decrypted_string)
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
        let decrypted = self
            .cipher()
            .decrypt(&self.nonce, ciphertext)
            .map_err(Error::UnableToDecrypt)?;

        Ok(decrypted)
    }
}
