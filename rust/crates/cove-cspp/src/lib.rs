//! Cloud Secure Passkey Protocol primitives
//!
//! CSPP is Cove's passkey-protected cloud backup protocol for Bitcoin wallet
//! data. It is intended to be useful beyond Cove: wallet apps should be able to
//! implement the protocol with platform-standard passkey, key storage, crypto,
//! and cloud storage APIs, without running a backup server or trusting the cloud
//! provider with plaintext wallet data.
//!
//! CSPP implements the original idea from
//! <https://praveenperera.com/blog/passkey-prf-bitcoin-wallet-backup/>, but the
//! implementation has drifted as expected. The structs and functions here are
//! the source of truth for Cove's current wire shape and key hierarchy.
//!
//! # Trust model
//!
//! CSPP separates key recovery from data storage:
//!
//! - A local [`master_key::MasterKey`] protects wallet backup records
//! - A passkey PRF output protects the cloud copy of that master key
//! - Cloud storage only receives encrypted JSON envelopes
//!
//! The platform layer is responsible for passkey authentication, local keychain
//! storage, and moving bytes to cloud storage. Rust prepares the backup records,
//! performs encryption and decryption, and tracks the record identifiers used by
//! iCloud or Google Drive app data.
//!
//! # Key hierarchy
//!
//! The root secret is a random 32-byte [`master_key::MasterKey`] generated with
//! the platform CSPRNG. It is stored locally through [`CsppStore`], which Cove
//! implements using the OS keychain or keystore. The local stored value is also
//! encrypted with a random `cove_util::Cryptor` so other keychain callers do not
//! accidentally receive plaintext master-key bytes.
//!
//! CSPP derives domain-separated values from the master key with HKDF-SHA256:
//!
//! - `cspp:v1:namespace-id` produces a deterministic cloud namespace ID
//! - `cspp:v1:critical` produces the critical data key used for wallet backups
//! - `cspp:v1:sensitive` exists for non-critical sensitive data
//!
//! The namespace ID is derived with HKDF-SHA256 using no salt, the master key as
//! input keying material, and `cspp:v1:namespace-id` as the info string. CSPP
//! takes the first 16 bytes of the expanded output and hex encodes them. The
//! result names the cloud backup directory without revealing the master key.
//! Current Cove cloud wallet backups encrypt the full [`backup_data::WalletEntry`]
//! with keys derived from the critical data key.
//!
//! # Cloud records
//!
//! CSPP stores two encrypted record types in cloud storage:
//!
//! - [`backup_data::EncryptedMasterKeyBackup`] wraps the 32-byte master key with
//!   a key produced by the selected passkey's PRF output
//! - [`backup_data::EncryptedWalletBackup`] wraps one encrypted wallet entry
//!
//! The master-key wrapper stores `version`, `prf_salt`, `nonce`, and
//! `ciphertext`. On restore, the app asks the passkey provider to evaluate the
//! PRF with `prf_salt`, then uses that 32-byte output to decrypt the master key.
//!
//! The wallet wrapper stores `version`, `wallet_salt`, `nonce`, and
//! `ciphertext`. The wallet key is derived with HKDF-SHA256 from the critical
//! data key and the random 32-byte `wallet_salt`, using the `cspp:v1:wallet`
//! info string. The decrypted [`backup_data::WalletEntry`] contains the wallet
//! secret, metadata JSON, descriptors, xpub, wallet mode, zstd-compressed BIP329
//! labels, label metadata, content revision hash, and update timestamp.
//!
//! # Main flows
//!
//! Local setup creates or loads the master key and keeps it in platform-backed
//! local storage. Cloud backup can be enabled later without changing local
//! wallet operation.
//!
//! Enabling cloud backup acquires passkey PRF material, encrypts and uploads the
//! master-key wrapper, derives the critical data key, then encrypts and uploads
//! each wallet backup record. Restore performs the inverse: download candidate
//! namespaces, authenticate with a passkey, decrypt the master key, derive the
//! critical data key, and decrypt wallet records.
//!
//! If the cloud passkey association breaks while the local master key is still
//! trusted, Cove can repair the master-key wrapper by creating or selecting a
//! passkey and re-wrapping the same master key. Wallet records do not need to be
//! rewritten for that repair because they are derived from the master key, not
//! directly from the passkey.
//!
//! # Cryptographic choices
//!
//! CSPP uses standard ChaCha20-Poly1305 instead of XChaCha20-Poly1305 so wallet
//! apps can implement it with platform-standard crypto APIs and avoid adding
//! export-compliance review burden for adopting the protocol.
//!
//! Wallet backup encryption compensates for the standard 96-bit nonce size by
//! deriving a fresh per-backup key. Each wallet envelope carries a random
//! 32-byte wallet salt, which is mixed with the critical data key before the
//! random nonce is used. Avoiding same-key nonce reuse therefore depends on
//! preserving both the random wallet salt and the random nonce in the envelope.
//!
//! The master-key wrapper is different: it is encrypted directly with the
//! passkey-PRF-derived wrapping key and a random nonce because that envelope is
//! written infrequently, normally once and only a few more times during repair
//! or reinitialization flows.

pub mod backup_data;
mod cspp;
pub mod error;
pub mod key_derivation;
pub mod master_key;
pub mod master_key_crypto;
mod serde_helpers;
pub mod store;
pub mod wallet_crypto;

pub use cspp::Cspp;
pub use error::CsppError;
pub use store::CsppStore;
