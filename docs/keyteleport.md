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

The clip lifetime is one minute. The UI test build shortens it to a few seconds, so instrumented tests can check the real clear.

## Master key encoding

Cove stores a wallet's master private key encoded for the wallet's network: xprv on mainnet, tprv on testnet, testnet4 and signet. File backup import, Cloud Backup restore, KeyTeleport receive and other key imports re-encode a key given in the other form. The version prefix does not change the key or its derivation, so Cove never refuses a backup over it.

Wallets restored by older versions can still hold the other form. Their keychain entry is left as it is, and Export Private Key, Cloud Backup upload and file backup export re-encode the key for the wallet's network when they read it. Devices that hold either form of a key therefore upload the same Cloud Backup revision. When enabling Cloud Backup merges an older namespace whose backup carries the other form, the merge expects the re-encoded revision, so cleanup still removes that namespace.

A KeyTeleport master key payload carries only the chain code and private key, so the transfer has no network. When sending, Cove re-encodes the key as an xprv, because the `keyteleport` crate accepts only that form, and the receiver stores it in its own wallet's form.

## Secret zeroization

Secret zeroization is best effort. The `WalletXprv` wrapper wipes its storage. Copies of `Xpriv` values in rust-bitcoin or BDK do not guarantee zeroization.

Decrypted TapSigner backup bytes use zeroizing storage inside the Rust keychain API. A plain byte copy is still required when data crosses the UniFFI boundary.

## Leaving the app

When Cove leaves the foreground, KeyTeleport hides any revealed receiver code, password, recovery words, or private key. It also clears a receiver code or sender password that was typed but not yet submitted. After unlocking, the user enters it again. Both platforms follow this rule.

The QR code shown next to a receiver code or password differs. iOS hides it too, so both are hidden after unlocking. Android goes back to showing the QR code, which is no use without the code or password beside it, and that stays hidden until the user reveals it again.

The receiver code only decrypts the receiver's one-time public key from the request QR, so it cannot decrypt a transfer on its own. Cove still clears it to keep both platforms consistent and to keep typed values off a locked screen.
