import SwiftUI

@Observable final class ScanManager {
    static let shared = ScanManager()

    private var app: AppManager {
        AppManager.shared
    }

    @MainActor
    func handleMultiFormat(_ multiFormat: MultiFormat) {
        do {
            try handleRecognizedMultiFormat(multiFormat)
        } catch {
            handleMultiFormatError(error)
        }
    }

    @MainActor
    private func handleRecognizedMultiFormat(_ multiFormat: MultiFormat) throws {
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
            handleReadyTapSigner(tapSigner)
        case let .bip329Labels(labels):
            try importLabels(labels)
        case let .keyTeleportReceiver(packet):
            handleKeyTeleportReceiver(packet)
        case let .keyTeleportSender(packet):
            handleKeyTeleportSender(packet)
        }
    }

    @MainActor
    private func handleReadyTapSigner(_ tapSigner: TapSigner) {
        if let wallet = app.findTapSignerWallet(tapSigner) {
            app.alertState = .init(.tapSignerWalletFound(walletId: wallet.id))
        } else {
            app.alertState = .init(.initializedTapSigner(tapSigner: tapSigner))
        }
    }

    @MainActor
    private func importLabels(_ labels: Bip329Labels) throws {
        guard let manager = app.walletManager,
              let selectedWallet = Database().globalConfig().selectedWallet(),
              selectedWallet == manager.id
        else {
            return setInvalidLabels()
        }

        try manager.importLabels(labels: labels)
        app.alertState = .init(.importedLabelsSuccessfully)
    }

    @MainActor
    private func handleMultiFormatError(_ error: Error) {
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

    @MainActor
    func handleKeyTeleportText(_ text: String) -> Bool {
        do {
            let multiFormat = try StringOrData(text).toMultiFormat()
            switch multiFormat {
            case .keyTeleportReceiver, .keyTeleportSender:
                handleMultiFormat(multiFormat)
                return true
            default:
                return false
            }
        } catch let error as MultiFormatError {
            return handleKeyTeleportTextError(error, text: text)
        } catch {
            return false
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
        if handleKeyTeleportUrl(url) {
            return
        }

        let fileHandler = FileHandler(filePath: url.absoluteString)

        do {
            let readResult = try fileHandler.read()
            try handleFileMultiFormat(readResult)
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

    @MainActor
    private func handleFileMultiFormat(_ multiFormat: MultiFormat) throws {
        switch multiFormat {
        case let .mnemonic(mnemonic):
            importHotWallet(mnemonic.words())
        case let .hardwareExport(export):
            importColdWallet(export)
        case let .address(addressWithNetwork):
            handleAddress(addressWithNetwork)
        case let .transaction(transaction):
            handleTransaction(transaction)
        case let .tapSignerUnused(tapSigner):
            app.sheetState = .init(.tapSigner(TapSignerRoute.initSelect(tapSigner)))
        case let .tapSignerReady(tapSigner):
            let panic =
                "TAPSIGNER not implemented \(tapSigner) doesn't make sense for file import"
            Log.error(panic)
        case let .bip329Labels(labels):
            guard let manager = app.walletManager,
                  let selectedWallet = Database().globalConfig().selectedWallet(),
                  selectedWallet == manager.id
            else {
                return setInvalidLabels()
            }

            try manager.importLabels(labels: labels)
        case let .signedPsbt(psbt):
            handleSignedPsbt(psbt)
        case let .keyTeleportReceiver(packet):
            handleKeyTeleportReceiver(packet)
        case let .keyTeleportSender(packet):
            handleKeyTeleportSender(packet)
        }
    }
}

extension ScanManager {
    @MainActor
    private func handleKeyTeleportTextError(_ error: MultiFormatError, text: String) -> Bool {
        if case .KeyTeleportPsbtNotSupported = error {
            app.alertState = TaggedItem(
                .invalidFormat(message: "KeyTeleport PSBT packets are not supported yet.")
            )
            return true
        }

        guard looksLikeKeyTeleportPacket(text) else {
            return false
        }

        app.alertState = TaggedItem(
            .invalidFormat(message: "This KeyTeleport packet could not be read.")
        )
        return true
    }

    private func looksLikeKeyTeleportPacket(_ text: String) -> Bool {
        let normalized = text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        return normalized.contains("keyteleport.com")
            || normalized.hasPrefix("b$2r")
            || normalized.hasPrefix("b$2s")
            || normalized.hasPrefix("b$2p")
    }

    @MainActor
    private func handleKeyTeleportUrl(_ url: URL) -> Bool {
        guard isKeyTeleportHost(url.host) else {
            return false
        }

        do {
            let multiFormat = try StringOrData(url.absoluteString).toMultiFormat()
            handleMultiFormat(multiFormat)
            return true
        } catch {
            app.alertState = .init(
                .invalidFormat(message: "This KeyTeleport link is not supported.")
            )
            return true
        }
    }

    private func isKeyTeleportHost(_ host: String?) -> Bool {
        guard let host = host?.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "."))
        else {
            return false
        }

        return host == "keyteleport.com" || host.hasSuffix(".keyteleport.com")
    }

    @MainActor
    private func handleKeyTeleportReceiver(_ packet: KeyTeleportReceiverPacket) {
        guard canIngestKeyTeleportPacket(requiring: .send) else {
            showKeyTeleportDirectionConflict(requiredDirection: .send)
            return
        }

        let manager = app.ensureKeyTeleportManager()
        manager.ingest(packet)
        app.navigateToKeyTeleport(.send)
    }

    @MainActor
    private func handleKeyTeleportSender(_ packet: KeyTeleportSenderPacket) {
        guard canIngestKeyTeleportPacket(requiring: .receive) else {
            showKeyTeleportDirectionConflict(requiredDirection: .receive)
            return
        }

        let manager = app.ensureKeyTeleportManager()
        manager.ingest(packet)
        app.navigateToKeyTeleport(.receive)
    }

    private func canIngestKeyTeleportPacket(
        requiring requiredDirection: KeyTeleportFlowDirection
    ) -> Bool {
        let routeDirections = [app.router.default.keyTeleportFlowDirection]
            + app.router.routes.map(\.keyTeleportFlowDirection)
        let activeDirections = routeDirections.compactMap(\.self)
            + [app.keyTeleportManager?.flowDirection].compactMap(\.self)

        return KeyTeleportPacketRoutingDecision.resolve(
            activeDirections: activeDirections,
            requiredDirection: requiredDirection
        ) == .accept
    }

    private func showKeyTeleportDirectionConflict(
        requiredDirection: KeyTeleportFlowDirection
    ) {
        let activeFlow = requiredDirection == .send ? "receive" : "send"
        let requestedFlow = requiredDirection == .send ? "send" : "receive"
        app.alertState = .init(
            .general(
                title: "KeyTeleport Session Active",
                message: "End the active \(activeFlow) session before starting a \(requestedFlow) session."
            )
        )
    }

    @MainActor
    private func importHotWallet(_ words: [String]) {
        do {
            let manager = ImportWalletManager()
            let walletMetadata = try manager.rust.importWallet(enteredWords: [words])
            try app.selectWalletOrThrow(walletMetadata.id)
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

            if app.walletManager?.id != id {
                try app.selectWalletOrThrow(id)
            }

            if app.walletManager?.id == id, app.walletManager?.walletMetadata.walletType != .hot {
                try app.walletManager?.rust.setWalletType(walletType: .cold)
            }
        } catch let WalletError.WalletAlreadyExists(id) {
            app.alertState = TaggedItem(.duplicateWallet(walletId: id))

            if (try? app.selectWalletOrThrow(id)) == nil {
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

        let route = RouteFactory().sendConfirmSignedTransaction(
            id: txnRecord.walletId(),
            details: txnRecord.confirmDetails(),
            transaction: transaction
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

        let route = RouteFactory().sendConfirmSignedPsbt(
            id: txnRecord.walletId(),
            details: txnRecord.confirmDetails(),
            psbt: psbt
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
