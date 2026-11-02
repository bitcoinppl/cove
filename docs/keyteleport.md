# KeyTeleport Security and Lifecycle Policy

## Session authorization

An unlocked Cove session is trusted. A KeyTeleport seed transfer does not require new authentication.

PIN or biometric prompts on other secret views are defense-in-depth and convenience controls. These prompts are not a Rust authorization boundary.

## Android clipboard cleanup

Sensitive clipboard cleanup on Android is best effort. Cove uses:

- a timer while the process runs
- persisted clipboard ownership and a cleanup deadline
- foreground cleanup after process death

Cove cannot guarantee clipboard deletion at an exact time.

## Master key encoding

Cove stores a wallet's master private key encoded for the wallet's network: xprv on mainnet, tprv on testnet, testnet4 and signet. File backup import, Cloud Backup restore, KeyTeleport receive and other key imports re-encode a key given in the other form. The version prefix does not change the key or its derivation, so Cove never refuses a backup over it.

Wallets restored by older versions can still hold the other form. Their keychain entry is left as it is, and Export Private Key re-encodes the key for the wallet's network when it reads it.

A KeyTeleport master key payload carries only the chain code and private key, so the transfer has no network. When sending, Cove re-encodes the key as an xprv, because the `keyteleport` crate accepts only that form, and the receiver stores it in its own wallet's form.

## Secret zeroization

Secret zeroization is best effort. The `WalletXprv` wrapper wipes its storage. Copies of `Xpriv` values in rust-bitcoin or BDK do not guarantee zeroization.

Decrypted TapSigner backup bytes use zeroizing storage inside the Rust keychain API. A plain byte copy is still required when data crosses the UniFFI boundary.
