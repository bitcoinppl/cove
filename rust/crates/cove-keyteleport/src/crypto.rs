use std::fmt;

use aes::Aes256;
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey, ecdh::shared_secret_point};
use ctr::cipher::{KeyIvInit as _, StreamCipher as _};
use pbkdf2::pbkdf2_hmac;
use rand::RngExt as _;
use sha2::{Digest as _, Sha256, Sha512};
use zeroize::Zeroize as _;

use crate::{Error, NumericCode, Result};

type Aes256Ctr = ctr::Ctr128BE<Aes256>;

const RECEIVER_CODE_DOMAIN: &[u8] = b"COLCARD4EVER";
const PBKDF2_ITERATIONS: u32 = 5000;

pub(crate) struct EphemeralPrivateKey([u8; 32]);

impl EphemeralPrivateKey {
    pub(crate) fn generate() -> Self {
        let mut rng = rand::rng();

        loop {
            let bytes = rng.random::<[u8; 32]>();

            if SecretKey::from_slice(&bytes).is_ok() {
                return Self(bytes);
            }
        }
    }

    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Result<Self> {
        SecretKey::from_slice(&bytes)?;

        Ok(Self(bytes))
    }

    pub(crate) fn expose_bytes(&self) -> [u8; 32] {
        self.0
    }

    pub(crate) fn public_key(&self) -> Result<PublicKey> {
        let secp = Secp256k1::new();
        let secret_key = self.secret_key()?;

        Ok(secret_key.public_key(&secp))
    }

    pub(crate) fn session_key(&self, public_key: &PublicKey) -> Result<SessionKey> {
        let secret_key = self.secret_key()?;
        let point = shared_secret_point(public_key, &secret_key);
        let digest = Sha256::digest(point);
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);

        Ok(SessionKey(bytes))
    }

    fn secret_key(&self) -> Result<SecretKey> {
        SecretKey::from_slice(&self.0).map_err(Into::into)
    }
}

impl fmt::Debug for EphemeralPrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EphemeralPrivateKey(****)")
    }
}

impl Drop for EphemeralPrivateKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub(crate) struct SessionKey([u8; 32]);

impl SessionKey {
    pub(crate) fn decrypt_outer(&self, body: &[u8]) -> Result<Vec<u8>> {
        decrypt_checked(&self.0, body)
    }

    pub(crate) fn encrypt_outer(&self, body: &[u8]) -> Vec<u8> {
        encrypt_checked(&self.0, body)
    }

    pub(crate) fn paranoid_key(&self, noid_key: &[u8; 5]) -> [u8; 32] {
        let mut key = [0_u8; 32];
        pbkdf2_hmac::<Sha512>(&self.0, noid_key, PBKDF2_ITERATIONS, &mut key);

        key
    }
}

impl fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionKey(****)")
    }
}

impl Drop for SessionKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub(crate) fn generate_receiver_packet(
    private_key: &EphemeralPrivateKey,
) -> Result<(NumericCode, [u8; 33])> {
    let public_key = private_key.public_key()?;
    let mut public_key_bytes = public_key.serialize();
    let hash = receiver_code_hash(private_key);

    public_key_bytes[0] ^= hash[20] & 0xfe;

    let numeric_value =
        u32::from_be_bytes(hash[4..8].try_into().expect("hash slice is 4 bytes")) % 100_000_000;
    let code = NumericCode::from_u32(numeric_value);

    let key = receiver_code_aes_key(&code);
    apply_aes256_ctr(&key, &mut public_key_bytes);

    Ok((code, public_key_bytes))
}

pub(crate) fn decrypt_receiver_pubkey(code: &NumericCode, payload: &[u8]) -> Result<PublicKey> {
    if payload.len() != 33 {
        return Err(Error::InvalidReceiverPacket);
    }

    let mut pubkey = [0_u8; 33];
    pubkey.copy_from_slice(payload);

    let key = receiver_code_aes_key(code);
    apply_aes256_ctr(&key, &mut pubkey);

    pubkey[0] &= 0x01;
    pubkey[0] |= 0x02;

    PublicKey::from_slice(&pubkey).map_err(Into::into)
}

pub(crate) fn encrypt_inner(paranoid_key: &[u8; 32], body: &[u8]) -> Vec<u8> {
    encrypt_checked(paranoid_key, body)
}

pub(crate) fn decrypt_inner(paranoid_key: &[u8; 32], body: &[u8]) -> Result<Vec<u8>> {
    decrypt_checked(paranoid_key, body)
}

fn receiver_code_hash(private_key: &EphemeralPrivateKey) -> [u8; 32] {
    let mut material = Vec::with_capacity(32 + RECEIVER_CODE_DOMAIN.len());
    material.extend_from_slice(&private_key.expose_bytes());
    material.extend_from_slice(RECEIVER_CODE_DOMAIN);

    let first = Sha256::digest(&material);
    material.zeroize();

    let second = Sha256::digest(first);
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&second);

    bytes
}

fn receiver_code_aes_key(code: &NumericCode) -> [u8; 32] {
    let digest = Sha256::digest(code.as_str().as_bytes());
    let mut key = [0_u8; 32];
    key.copy_from_slice(&digest);

    key
}

fn encrypt_checked(key: &[u8; 32], body: &[u8]) -> Vec<u8> {
    let mut ciphertext = body.to_vec();
    apply_aes256_ctr(key, &mut ciphertext);
    ciphertext.extend_from_slice(&checksum(body));

    ciphertext
}

fn decrypt_checked(key: &[u8; 32], body: &[u8]) -> Result<Vec<u8>> {
    if body.len() < 3 {
        return Err(Error::Checksum);
    }

    let (ciphertext, expected_checksum) = body.split_at(body.len() - 2);
    let mut plaintext = ciphertext.to_vec();
    apply_aes256_ctr(key, &mut plaintext);

    if checksum(&plaintext) != expected_checksum {
        return Err(Error::Checksum);
    }

    Ok(plaintext)
}

fn checksum(body: &[u8]) -> [u8; 2] {
    let digest = Sha256::digest(body);

    [digest[30], digest[31]]
}

fn apply_aes256_ctr(key: &[u8; 32], body: &mut [u8]) {
    let iv = [0_u8; 16];
    let mut cipher = Aes256Ctr::new(key.into(), &iv.into());
    cipher.apply_keystream(body);
}
