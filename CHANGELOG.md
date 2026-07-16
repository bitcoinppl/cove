# Changelog

## [Unreleased]

## [1.3.0] - 2026-07-16

- Added encrypted cloud backup, onboarding restore, and backup management on iOS and Android
- Improved wallet recovery with progressive scanning, custom gap limits, and richer backup metadata
- Added Payjoin URI support, clearer incoming balance reporting, and broader wallet import compatibility
- Improved security for seed screens, imported PSBTs, local app data, and TAPSIGNER backups
- Improved startup, connectivity, cloud sync, hardware-wallet exports, and cross-platform reliability

## [1.2.5] - 2026-05-29

- Clarified Android version information in Settings

## [1.2.4] - 2026-05-29

- Fixed an Android send-flow crash when PIN or biometric authentication was enabled

## [1.2.3] - 2026-05-19

- Fixed imports for legacy and wrapped SegWit hot wallets

## [1.2.2] - 2026-03-20

- Added recovery flows for wallets missing seed words after a device restore
- Added seed-word import to compatible existing wallets
- Improved fee estimates, wallet scanning, address formatting, and Sparrow descriptor exports
- Fixed an iOS navigation freeze affecting some accessibility text settings

## [1.2.1] - 2026-02-07

- Fixed a crash in the send flow

## [1.2.0] - 2026-02-06

- Added historical fiat values to transaction details and send confirmation
- Added public descriptor and SeedQR exports
- Added support for signed, unfinalized PSBTs from more hardware wallets
- Improved wallet deletion safeguards, PIN privacy, biometric unlocking, and seed-word imports
- Preserved wallet scroll position when returning from transaction details

## [1.1.0] - 2025-12-23

- Added animated UR QR support and improved animated QR performance
- Added QR and share options for label exports
- Improved wallet switching, transaction loading, and send-flow startup
- Refined wallet import and cross-platform UI behavior

## [1.0.3] - 2025-12-02

- Fixed deadlocks when verifying seed words and renaming wallets

## [1.0.2] - 2025-11-24

- Expanded Android support across wallet management, sending, settings, labels, authentication, and TAPSIGNER flows
- Added receive-address and wallet-operation QR scanning improvements
- Improved iOS 26 compatibility and cross-platform UI consistency
- Improved NFC reliability and prevented repeated TAPSIGNER import prompts

## [1.0.1] - 2025-06-16

- Improved TAPSIGNER setup and NFC error handling
- Fixed seed verification and backup progress being lost when the app entered the background
- Added the full address derivation path and standardized loading indicators

## [1.0.0] - 2025-06-11

- Improved seed import and receive layouts across screen sizes
- Fixed duplicate-wallet and seed-length validation feedback
- Standardized wallet balances on spendable funds
- Improved pending transaction refresh behavior

## [0.5.1] - 2025-06-03

- Added the required Terms and Conditions prompt
- Fixed stale labels in UTXO and transaction details
- Improved seed-word validation during wallet import

## [0.5.0] - 2025-05-28

- Added coin control with UTXO search, filtering, sorting, selection, and custom send amounts
- Added UTXO labels to send confirmation
- Added warnings and safeguards for disproportionately high fees

## [0.4.1] - 2025-05-12

- Redesigned receive-address and advanced send-detail sheets
- Improved large fiat amount handling, address validation, keyboard behavior, and Signet links

## [0.4.0] - 2025-05-08

- Added hardware-wallet imports using key expressions, including Krux support
- Added transaction CSV exports with historical fiat values
- Added Testnet4 support
- Improved send entry, fee consistency, and overall send-flow reliability

## [0.3.0] - 2025-04-07

- Added TAPSIGNER setup, import, PIN changes, backups, and PSBT signing
- Added automatic updates for pending transactions

## [0.2.2] - 2025-03-11

- Improved decoy PIN reliability and plausible deniability
- Fixed NFC transaction import and scanning lockups
- Fixed unsigned transaction visibility and imported wallet names

## [0.2.1] - 2025-03-07

- Improved decoy PIN plausible deniability and PIN settings usability

## [0.2.0] - 2025-03-05

- Added watch-only hardware-wallet imports from extended public keys
- Fixed transaction list appearance in dark mode

## [0.1.0] - 2025-03-04

- Added app version, build, and feedback details to wallet settings
- Fixed crashes caused by custom node URLs using HTTP or HTTPS

## [0.0.1] - 2025-02-28

- Added hot and hardware wallet creation and import
- Added Bitcoin sending, hardware-wallet signing, and transaction details
- Added multiple wallets, BIP329 labels, fiat currency selection, and custom nodes
- Added Face ID, PIN locking, decoy PINs, and wipe PINs
