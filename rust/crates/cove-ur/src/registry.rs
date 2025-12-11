//! CBOR tag assignments from BCR-2020-006
//! See: https://github.com/BlockchainCommons/Research/blob/master/papers/bcr-2020-006-urtypes.md

/// crypto-seed tag (BCR-2020-006)
pub const CRYPTO_SEED: u64 = 300;

/// crypto-hdkey tag (BCR-2020-007)
pub const CRYPTO_HDKEY: u64 = 303;

/// crypto-keypath tag (BCR-2020-007)
pub const CRYPTO_KEYPATH: u64 = 304;

/// crypto-coin-info tag (BCR-2020-007)
pub const CRYPTO_COIN_INFO: u64 = 305;

/// crypto-output tag (BCR-2020-010)
pub const CRYPTO_OUTPUT: u64 = 308;

/// crypto-psbt tag (BCR-2020-006)
pub const CRYPTO_PSBT: u64 = 310;

/// crypto-account tag (BCR-2020-015)
pub const CRYPTO_ACCOUNT: u64 = 311;

// Script expression tags (BCR-2020-010)
/// script-hash (sh) - P2SH wrapper
pub const SCRIPT_HASH: u64 = 400;

/// witness-script-hash (wsh) - P2WSH
pub const WITNESS_SCRIPT_HASH: u64 = 401;

/// pay-to-pubkey-hash (pkh) - P2PKH (BIP44)
pub const PAY_TO_PUBKEY_HASH: u64 = 403;

/// witness-pubkey-hash (wpkh) - P2WPKH (BIP84)
pub const WITNESS_PUBKEY_HASH: u64 = 404;

/// taproot (tr) - P2TR (BIP86)
pub const TAPROOT: u64 = 409;

// =============================================================================
// CBOR Map Keys
// =============================================================================

/// CBOR map keys for crypto-hdkey (BCR-2020-007)
pub mod hdkey_keys {
    pub const IS_MASTER: u32 = 1;
    pub const IS_PRIVATE: u32 = 2;
    pub const KEY_DATA: u32 = 3;
    pub const CHAIN_CODE: u32 = 4;
    pub const USE_INFO: u32 = 5;
    pub const ORIGIN: u32 = 6;
    pub const CHILDREN: u32 = 7;
    pub const PARENT_FINGERPRINT: u32 = 8;
    pub const NAME: u32 = 9;
    pub const SOURCE: u32 = 10;
}

/// CBOR map keys for crypto-seed (BCR-2020-006)
pub mod seed_keys {
    pub const PAYLOAD: u32 = 1;
    pub const CREATION_DATE: u32 = 2;
    pub const NAME: u32 = 3;
    pub const NOTE: u32 = 4;
}

/// CBOR map keys for crypto-keypath (BCR-2020-007)
pub mod keypath_keys {
    pub const COMPONENTS: u32 = 1;
    pub const SOURCE_FINGERPRINT: u32 = 2;
    pub const DEPTH: u32 = 3;
}

/// CBOR map keys for crypto-coin-info (BCR-2020-007)
pub mod coin_info_keys {
    pub const COIN_TYPE: u32 = 1;
    pub const NETWORK: u32 = 2;
}

/// CBOR map keys for crypto-account (BCR-2020-015)
pub mod account_keys {
    pub const MASTER_FINGERPRINT: u32 = 1;
    pub const OUTPUT_DESCRIPTORS: u32 = 2;
}

// =============================================================================
// Data Lengths
// =============================================================================

/// Common byte lengths for cryptographic data
pub mod lengths {
    /// Compressed public key (33 bytes)
    pub const COMPRESSED_PUBKEY: usize = 33;
    /// Private key / chain code (32 bytes)
    pub const PRIVATE_KEY: usize = 32;
    /// Chain code (32 bytes)
    pub const CHAIN_CODE: usize = 32;
    /// Fingerprint (4 bytes)
    pub const FINGERPRINT: usize = 4;
}

/// Valid BIP39 entropy lengths in bytes
pub const VALID_BIP39_ENTROPY_LENGTHS: [usize; 5] = [16, 20, 24, 28, 32];

// =============================================================================
// CBOR Major Types (RFC 8949)
// =============================================================================

/// CBOR major type indicators (high 3 bits of initial byte)
pub mod cbor_type {
    /// Map with 0-23 items (0xa0-0xb7)
    pub const MAP_SMALL_MIN: u8 = 0xa0;
    pub const MAP_SMALL_MAX: u8 = 0xb7;

    /// Map with 1-byte length follows (0xb8)
    pub const MAP_1BYTE_LEN: u8 = 0xb8;
    /// Map with 2-byte length follows (0xb9)
    pub const MAP_2BYTE_LEN: u8 = 0xb9;
    /// Map with 4-byte length follows (0xba)
    pub const MAP_4BYTE_LEN: u8 = 0xba;
    /// Map with 8-byte length follows (0xbb)
    pub const MAP_8BYTE_LEN: u8 = 0xbb;

    /// Check if a byte indicates a CBOR map
    pub fn is_map(byte: u8) -> bool {
        (MAP_SMALL_MIN..=MAP_SMALL_MAX).contains(&byte)
            || byte == MAP_1BYTE_LEN
            || byte == MAP_2BYTE_LEN
            || byte == MAP_4BYTE_LEN
            || byte == MAP_8BYTE_LEN
    }
}
