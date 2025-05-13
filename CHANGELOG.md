# Changelog

## [Unreleased]

## [0.4.1] - 2025-05-12

### Changes

- Redesigned address QR receive sheet
- Redesigned "Advanced More Details" sheet in the send flow

### Fixes

- Fixed a bug where entering a comically large fiat amount would crash the app
- Fixed a bug where in Testnet4 addresses were coming up as invalid when they weren't
- Fixed a bug where keyboard wasn't dismissing when address was valid
- Fixed signet transaction viewer link

## [0.4.0] - 2025-05-08

### Features

- Support for importing hardware wallets with [Key Expressions](https://github.com/bitcoin/bips/blob/master/bip-0380.mediawiki#key-expressions)
  - This adds support for using Cove with [Krux](https://github.com/selfcustody/krux)
- Create a CSV file of your transactions with fiat values for each transaction at the time of the transaction
  - This is useful for tax reporting
- Send flow improvements and bug fixes
  - Show proper formatting as you type
  - Default to entering the amount in before the address
  - Small UI bug fixes
- Testnet4 support

### Fixes

- Fix bug where the total fees the user sees changes between screens

### Internal (non-user facing)

- Extracted out multiple modules into their own crates
- Updated build script to work with multiple crates and to create a SPM package for iOS
- Complete refactor of send flow, moved logic over to rust

## [0.3.0] - 2025-04-07

### TAPSIGNER Support

- Setup TAPSIGNER
- Change TAPSIGNER PIN
- Backup TAPSIGNER
- Import TAPSIGNER public key
- Sign PSBT using TAPSIGNER

### Features / Improvements

- Pending transactions are automatically updated without user having to refresh

## [0.2.2] - 2025-03-11

### Fixes

- Fixed bug where it was possible to get stuck in "Decoy PIN" mode
- Added more plausible deniability to decoy PIN mode
- Fix bug where unsigned transactions were not showing up in the transaction list
- Fixed bug where NFC signed txn import was showing an error even tho it was successful
- Fixed bug where NFC scanning would lock the app
- Fix where imported name was reset to the default name

### Changes

- Don't show cover when using NFC

## [0.2.1] - 2025-03-07

### Fixes

- Add more plausible deniability to decooy PIN mode
  - Pretend to change PIN and trick PINs in the settings scree
- Make it easier to click the "Change PIN" button in the settings screen

## [0.2.0] - 2025-03-05

### Features

- Add ability to import an XPUB (not a descriptor) as a hardware wallet

### Fixes

- Fixed visual bug in dark mode transaction list (main wallet screen)

## [0.1.0] - 2025-03-04

- Add version number, git short hash and feedback email to wallet settings screen
- Fixed bug where custom node url starting with `http` or `https` would crash the app

## [0.0.1] [Build 39] - 2025-02-28

- Import hardware wallet (xpub / public descriptor) using NFC, File & QR
- Create hot wallet, and verify hot wallet backup
- Send Bitcoin using a hot wallet
- Send Bitcoin using a hardware wallet, using NFC, File or QR for transferring PSBTs
- View transaction details
- Create and use multiple wallets
- Create and use BIP329 labels
- Import and Export BIP329 labels
- Select your preferred fiat currency
- Connect your own node
- Create Trick Pins (Wipe Data & Decoy PIN)
- Use FaceID or PIN to lock your wallets
