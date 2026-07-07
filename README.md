<div align="center">
  <img src="images/cove_logo_github.jpg" width="250" >
</div>

## About

Cove is a simple to use yet powerful Bitcoin mobile wallet.
The wallet is built on top of the [BDK](https://bitcoindevkit.org/) library.

We provide hot wallet support but one of the main goals is to be the best mobile wallet to use with hardware wallets.

## Available on iOS and Android

<p>
  <a href="https://covebitcoinwallet.com/appstore"><img alt="Download on the App Store" src="https://github.com/user-attachments/assets/118e679c-a205-4251-988a-107c4ee78076" height="60"></a>
  <a href="https://play.google.com/store/apps/details?id=org.bitcoinppl.cove"><img alt="Get it on Google Play" src="https://upload.wikimedia.org/wikipedia/commons/7/78/Google_Play_Store_badge_EN.svg" width="198"></a>
</p>

## Build from Source

See [CONTRIBUTING.md](CONTRIBUTING.md) for prerequisites and build instructions.

## Documentation

- [CONTRIBUTING.md](CONTRIBUTING.md) - Development setup, commands, workflow
- [SECURITY.md](SECURITY.md) - Private vulnerability reporting
- [ARCHITECTURE.md](ARCHITECTURE.md) - System design and codebase structure
- [docs/ios_android_parity.md](docs/ios_android_parity.md) - iOS/Android UI patterns
- [docs/icloud_drive.md](docs/icloud_drive.md) - iCloud Drive behavior and file coordination notes
- [docs/passkeys.md](docs/passkeys.md) - Passkey behavior and Cloud Backup confirmation notes

## Features

![features list](images/features.png)

- Create and import hot wallets, verify recovery words, and use SeedQR backup and restore
- Import watch-only and hardware wallets from xpubs, public descriptors, and key expressions using NFC, files, or QR codes
- Use Coldcard, TAPSIGNER, Krux, Jade, SeedSigner, Foundation Passport, and other PSBT-compatible hardware wallets
- Set up, import, back up, change PIN, and sign with TAPSIGNER over NFC
- Send Bitcoin with hot wallets or hardware wallets using QR (BBQr and UR), NFC, or files for PSBT transfer
- Choose custom fee rates and review fee warnings before sending
- Create and manage multiple wallets across Bitcoin, Signet, Testnet, and Testnet4
- Use Native Segwit, Wrapped Segwit, and Legacy address types
- View receive addresses, transaction details, UTXOs used, and block explorer links
- Search, sort, filter, lock, unlock, label, and select individual UTXOs for coin control
- Create, import, and export BIP329 labels for wallets, transactions, and UTXOs
- Export transaction CSVs with historical fiat prices for tax reporting
- Select your preferred fiat currency and Bitcoin unit
- Connect to your own Electrum or Esplora node
- Enable end-to-end encrypted Cloud Backup and restore protected by passkeys, using iCloud Drive on iOS and Google Drive on Android
- Use PIN or biometric app lock, plus Wipe Data and Decoy trick PINs

## Demo Video

https://github.com/user-attachments/assets/9c933b90-a991-4c09-be29-2825d535bc1e

## Coming Soon

- Full SATSCARD support

## Acknowledgements

- [OpenSats](https://opensats.org/) for the grant that made it possible for me to dedicate my time to this project.
- [BDK](https://bitcoindevkit.org/) which Cove is built on, thanks for the great work, and for your help along the way.
- [Adrian Lischer](https://x.com/adrianlischer) for the UI designs and UX feedback.
- All the alpha and beta testers that have provided valuable feedback.
- [Craig Raw](https://x.com/craigraw) for helping me make integrations with sparrow work smoothly.
- [Coinkite](http://coinkite.com) for providing me with hardware to test on and helping me with integrations.
- [NVK](http://twitter.com/nvk) for the Cove name and feedback.

## License

Cove is released under the [MIT license](LICENSE).
