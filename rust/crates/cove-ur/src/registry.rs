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
