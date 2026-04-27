import SwiftUI

@Observable final class ScanManager {
    static let shared = ScanManager()

    private var app: AppManager {
        AppManager.shared
    }

    @MainActor
    func handleMultiFormat(_ multiFormat: MultiFormat) {
        do {
            switch multiFormat {
            case let .mnemonic(mnemonic):
                importHotWallet(mnemonic.words())
            case let .hardwareExport(export):
                importColdWallet(export)
            case let .address(addressWithNetwork):
                handleAddress(addressWithNetwork)
            case let .transaction(transaction):
                handleTransaction(transaction)
            case let .signedPsbt(psbt):
                handleSignedPsbt(psbt)
            case let .tapSignerUnused(tapSigner):
                app.alertState = .init(.uninitializedTapSigner(tapSigner: tapSigner))
            case let .tapSignerReady(tapSigner):
                if let wallet = app.findTapSignerWallet(tapSigner) {
                    app.alertState = .init(.tapSignerWalletFound(walletId: wallet.id))
                } else {
                    app.alertState = .init(.initializedTapSigner(tapSigner: tapSigner))
                }
            case let .bip329Labels(labels):
                guard let manager = app.walletManager else { return setInvalidLabels() }
                guard let selectedWallet = Database().globalConfig().selectedWallet() else {
                    return setInvalidLabels()
                }

                try LabelManager(id: selectedWallet).import(labels: labels)
                app.alertState = .init(.importedLabelsSuccessfully)
                Task { await manager.rust.getTransactions() }
            }
        } catch {
            switch error {
            case let multiFormatError as MultiFormatError:
                Log.error(
                    "MultiFormat not recognized: \(multiFormatError): \(multiFormatError.description)"
                )
                app.alertState = TaggedItem(.invalidFormat(message: multiFormatError.description))

            default:
                Log.error("Unable to handle scanned code, error: \(error)")
                app.alertState = TaggedItem(.invalidFileFormat(message: error.localizedDescription))
            }
        }
    }

    @MainActor
    func handleNfcScan(_ nfcMessage: NfcMessage) {
        do {
            let multiFormat = try nfcMessage.tryIntoMultiFormat()
            handleMultiFormat(multiFormat)
        } catch {
            switch error {
            case let multiFormatError as MultiFormatError:
                Log.error(
                    "MultiFormat not recognized: \(multiFormatError): \(multiFormatError.description)"
                )
                app.alertState = TaggedItem(.invalidFormat(message: multiFormatError.description))

            default:
                Log.error("Unable to handle scanned code, error: \(error)")
                app.alertState = TaggedItem(.invalidFileFormat(message: error.localizedDescription))
            }
        }
    }

    @MainActor
    func handleFileOpen(_ url: URL) {
        let fileHandler = FileHandler(filePath: url.absoluteString)

        do {
            let readResult = try fileHandler.read()
            switch readResult {
            case let .mnemonic(mnemonic):
                importHotWallet(mnemonic.words())
            case let .hardwareExport(export):
                importColdWallet(export)
            case let .address(addressWithNetwork):
                handleAddress(addressWithNetwork)
            case let .transaction(txn):
                handleTransaction(txn)
            case let .tapSignerUnused(tapSigner):
                app.sheetState = .init(.tapSigner(TapSignerRoute.initSelect(tapSigner)))
            case let .tapSignerReady(tapSigner):
                let panic =
                    "TAPSIGNER not implemented \(tapSigner) doesn't make sense for file import"
                Log.error(panic)
            case let .bip329Labels(labels):
                if let selectedWallet = Database().globalConfig().selectedWallet() {
                    return try LabelManager(id: selectedWallet).import(labels: labels)
                }

                app.alertState = TaggedItem(
                    .invalidFileFormat(
                        message:
                        "Currently BIP329 labels must be imported through the wallet actions"
                    )
                )
            case let .signedPsbt(psbt):
                handleSignedPsbt(psbt)
            }
        } catch {
            switch error {
            case let FileHandlerError.NotRecognizedFormat(multiFormatError):
                Log.error("Unrecognized format multi format error: \(multiFormatError)")
                app.alertState = TaggedItem(
                    .invalidFileFormat(message: multiFormatError.localizedDescription)
                )

            case let FileHandlerError.OpenFile(error):
                Log.error("File handler error: \(error)")

            case let FileHandlerError.ReadFile(error):
                Log.error("Unable to read file: \(error)")

            case FileHandlerError.FileNotFound:
                Log.error("File not found")

            default:
                Log.error("Unknown error file handling file: \(error)")
            }
        }
    }
}

extension ScanManager {
    @MainActor
    private func importHotWallet(_ words: [String]) {
        do {
            let manager = ImportWalletManager()
            let walletMetadata = try manager.rust.importWallet(enteredWords: [words])
            try app.rust.selectWallet(id: walletMetadata.id)
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                app.alertState = TaggedItem(.invalidWordGroup)
            case let .WalletAlreadyExists(walletId):
                Log.warn("Attempted to import words for an existing hot wallet: \(walletId)")
                app.alertState = TaggedItem(.duplicateWallet(walletId: walletId))
            default:
                Log.error("Unable to import wallet: \(error)")
                app.alertState = TaggedItem(
                    .errorImportingHotWallet(message: error.localizedDescription)
                )
            }
        } catch {
            Log.error("Unknown error \(error)")
            app.alertState = TaggedItem(
                .errorImportingHotWallet(message: error.localizedDescription)
            )
        }
    }

    @MainActor
    private func importColdWallet(_ export: HardwareExport) {
        do {
            let wallet = try Wallet.newFromExport(export: export)
            let id = wallet.id()
            Log.debug("Imported Wallet: \(id)")
            app.alertState = TaggedItem(.importedSuccessfully)

            if app.walletManager?.id != id { try app.rust.selectWallet(id: id) }

            if app.walletManager?.id == id, app.walletManager?.walletMetadata.walletType != .hot {
                try app.walletManager?.rust.setWalletType(walletType: .cold)
            }
        } catch let WalletError.WalletAlreadyExists(id) {
            app.alertState = TaggedItem(.duplicateWallet(walletId: id))

            if (try? app.rust.selectWallet(id: id)) == nil {
                app.alertState = TaggedItem(.unableToSelectWallet)
            }
        } catch {
            app.alertState = TaggedItem(
                .errorImportingHardwareWallet(message: error.localizedDescription)
            )
        }
    }

    @MainActor
    private func handleAddress(_ addressWithNetwork: AddressWithNetwork) {
        let currentNetwork = Database().globalConfig().selectedNetwork()
        let address = addressWithNetwork.address()
        let network = addressWithNetwork.network()
        let selectedWallet = Database().globalConfig().selectedWallet()

        if selectedWallet == nil {
            app.alertState = TaggedItem(AppAlertState.noWalletSelected(address: address))
            return
        }

        if !addressWithNetwork.isValidForNetwork(network: currentNetwork) {
            app.alertState = TaggedItem(
                AppAlertState.addressWrongNetwork(
                    address: address, network: network, currentNetwork: currentNetwork
                )
            )
            return
        }

        let amount = addressWithNetwork.amount()
        app.alertState = TaggedItem(.foundAddress(address: address, amount: amount))
    }

    @MainActor
    private func handleTransaction(_ transaction: BitcoinTransaction) {
        Log.debug(
            "Received BitcoinTransaction: \(transaction): \(transaction.txIdHash())"
        )

        let db = Database().unsignedTransactions()
        let txnRecord = db.getTx(txId: transaction.txId())

        guard let txnRecord else {
            Log.error("No unsigned transaction found for \(transaction.txId())")
            app.alertState = .init(.noUnsignedTransactionFound(txId: transaction.txId()))
            return
        }

        let route = RouteFactory().sendConfirm(
            id: txnRecord.walletId(), details: txnRecord.confirmDetails(),
            signedTransaction: transaction
        )

        app.pushRoute(route)
    }

    @MainActor
    private func handleSignedPsbt(_ psbt: Psbt) {
        Log.debug("Received signed PSBT: \(psbt.txId())")

        let db = Database().unsignedTransactions()
        let txnRecord = db.getTx(txId: psbt.txId())

        guard let txnRecord else {
            Log.error("No unsigned transaction found for PSBT \(psbt.txId())")
            app.alertState = .init(.noUnsignedTransactionFound(txId: psbt.txId()))
            return
        }

        let route = RouteFactory().sendConfirm(
            id: txnRecord.walletId(), details: txnRecord.confirmDetails(),
            signedPsbt: psbt
        )

        app.pushRoute(route)
    }

    @MainActor
    private func setInvalidLabels() {
        app.alertState = TaggedItem(
            .invalidFileFormat(
                message: "Currently BIP329 labels must be imported through the wallet actions"
            )
        )
    }
}
