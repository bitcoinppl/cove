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

## Secret zeroization

Secret zeroization is best effort. The `WalletXprv` wrapper wipes its storage. Copies of `Xpriv` values in rust-bitcoin or BDK do not guarantee zeroization.

Decrypted TapSigner backup bytes use zeroizing storage inside the Rust keychain API. A plain byte copy is still required when data crosses the UniFFI boundary.
