use aes::Aes256;
use aes::cipher::{KeyIvInit as _, StreamCipher as _};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};
use ctr::Ctr128BE;
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest as _, Sha256, Sha512};

/// Derive the shared session key using ECDH.
///
/// Per the Key Teleport spec: SHA256(X || Y) of the shared point where X and Y
/// are the full uncompressed coordinates (64 bytes total).
pub(crate) fn session_key(local_privkey: &SecretKey, remote_pubkey: &PublicKey) -> [u8; 32] {
    let secp = Secp256k1::new();
    let scalar = Scalar::from_be_bytes(local_privkey.secret_bytes())
        .expect("secret key bytes are always a valid scalar");
    let point = remote_pubkey.mul_tweak(&secp, &scalar).expect("valid EC multiplication");
    // serialize_uncompressed: 04 || X(32) || Y(32) — drop the 04 prefix
    let uncompressed = point.serialize_uncompressed();
    Sha256::digest(&uncompressed[1..]).into()
}

/// AES-256-CTR encrypt or decrypt (same operation — XOR keystream).
/// Zero IV as specified by the Key Teleport protocol.
pub(crate) fn aes256ctr(key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    let iv = [0u8; 16];
    let mut cipher = Ctr128BE::<Aes256>::new(key.into(), &iv.into());
    let mut out = data.to_vec();
    cipher.apply_keystream(&mut out);
    out
}

/// Derive the AES key used to encrypt/decrypt the receiver's pubkey in the R packet.
/// Key = SHA256(zero-padded 8-digit decimal string of the numeric code).
pub(crate) fn receiver_pubkey_key(numeric_code: u32) -> [u8; 32] {
    let code_str = format!("{:08}", numeric_code);
    Sha256::digest(code_str.as_bytes()).into()
}

/// Stretch the teleport password using PBKDF2-SHA512.
/// Per spec: password = session_key, salt = teleport_pass, iter = 5000.
/// Returns the upper 256 bits (first 32 bytes) of the 512-bit output.
pub(crate) fn pbkdf2_stretch(session_key: &[u8; 32], teleport_pass: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 64];
    pbkdf2_hmac::<Sha512>(session_key, teleport_pass, 5000, &mut out);
    out[..32].try_into().expect("32 bytes from 64-byte output")
}

/// 2-byte checksum: last 2 bytes of SHA256(data).
pub(crate) fn checksum(data: &[u8]) -> [u8; 2] {
    let hash = Sha256::digest(data);
    [hash[30], hash[31]]
}

/// Verify the 2-byte checksum appended to `data_with_checksum`.
/// Returns the payload bytes (without checksum) if valid.
pub(crate) fn verify_checksum(data_with_checksum: &[u8]) -> Option<&[u8]> {
    if data_with_checksum.len() < 2 {
        return None;
    }
    let (body, cs) = data_with_checksum.split_at(data_with_checksum.len() - 2);
    let expected = checksum(body);
    if cs == expected { Some(body) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;

    #[test]
    fn session_key_is_symmetric() {
        use rand::RngExt as _;
        let secp = Secp256k1::new();
        let mut bytes_a = [0u8; 32];
        let mut bytes_b = [0u8; 32];
        rand::rng().fill(&mut bytes_a);
        rand::rng().fill(&mut bytes_b);
        let sk_a = SecretKey::from_slice(&bytes_a).unwrap();
        let sk_b = SecretKey::from_slice(&bytes_b).unwrap();
        let pk_a = sk_a.public_key(&secp);
        let pk_b = sk_b.public_key(&secp);

        let ka = session_key(&sk_a, &pk_b);
        let kb = session_key(&sk_b, &pk_a);
        assert_eq!(ka, kb, "ECDH must be symmetric");
    }

    #[test]
    fn aes256ctr_roundtrip() {
        let key = [0x42u8; 32];
        let plain = b"hello key teleport";
        let cipher = aes256ctr(&key, plain);
        let recovered = aes256ctr(&key, &cipher);
        assert_eq!(recovered, plain);
    }

    #[test]
    fn checksum_verify_roundtrip() {
        let data = b"some payload data";
        let cs = checksum(data);
        let mut with_cs = data.to_vec();
        with_cs.extend_from_slice(&cs);
        assert_eq!(verify_checksum(&with_cs), Some(data.as_slice()));
    }

    #[test]
    fn checksum_detects_corruption() {
        let data = b"some payload data";
        let cs = checksum(data);
        let mut with_cs = data.to_vec();
        with_cs.extend_from_slice(&cs);
        with_cs[0] ^= 0xFF;
        assert_eq!(verify_checksum(&with_cs), None);
    }

    #[test]
    fn receiver_pubkey_key_is_deterministic() {
        assert_eq!(receiver_pubkey_key(12345678), receiver_pubkey_key(12345678));
        assert_ne!(receiver_pubkey_key(12345678), receiver_pubkey_key(99999999));
    }
}
