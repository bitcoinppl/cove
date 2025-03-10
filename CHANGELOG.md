# Changelog

## [Unreleased]

### Fixes

- Add bug where it was possible to get stuck in "Decoy PIN" mode
- Add more plausible deniability to decooy PIN mode

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
