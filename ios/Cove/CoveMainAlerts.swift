import SwiftUI

extension AppAlertState: TaggedAlertPresentable {
    func alert(context: CoveMainPresentationContext) -> AnyAlertBuilder {
        AlertBuilder(
            title: title(),
            message: {
                AppAlertMessage(alert: self)
            },
            actions: {
                AppAlertActions(alert: self, context: context)
            }
        ).eraseToAny()
    }
}

private struct AppAlertMessage: View {
    let alert: AppAlertState

    var body: some View {
        let text = alert.message()

        if case .foundAddress = alert {
            Text(text.map { "\($0)\u{200B}" }.joined())
                .font(.system(.caption2, design: .monospaced))
                .minimumScaleFactor(0.5)
                .lineLimit(2)
        } else {
            Text(text)
        }
    }
}

private struct AppAlertActions: View {
    let alert: AppAlertState
    let context: CoveMainPresentationContext

    var body: some View {
        switch alert.actionGroup {
        case .wallet:
            WalletAlertActions(alert: alert, app: context.app)

        case .address:
            AddressAlertActions(alert: alert, app: context.app)

        case .watchOnly:
            WatchOnlyAlertActions(alert: alert, app: context.app)

        case .tapSigner:
            TapSignerAlertActions(alert: alert, app: context.app)

        case .dismiss:
            DismissAlertActions(app: context.app)
        }
    }
}

private enum AppAlertActionGroup {
    case wallet
    case address
    case watchOnly
    case tapSigner
    case dismiss
}

private extension AppAlertState {
    var actionGroup: AppAlertActionGroup {
        switch self {
        case .duplicateWallet, .hotWalletKeyMissing, .confirmWatchOnly, .walletDatabaseCorrupted:
            .wallet

        case .addressWrongNetwork, .noWalletSelected, .foundAddress, .noCameraPermission:
            .address

        case .cantSendOnWatchOnlyWallet, .watchOnlyImportHardware, .watchOnlyImportWords:
            .watchOnly

        case .uninitializedTapSigner,
             .tapSignerWalletFound,
             .initializedTapSigner,
             .tapSignerNoBackup,
             .tapSignerWrongPin:
            .tapSigner

        case .invalidWordGroup,
             .errorImportingHotWallet,
             .importedSuccessfully,
             .unableToSelectWallet,
             .errorImportingHardwareWallet,
             .invalidFileFormat,
             .importedLabelsSuccessfully,
             .unableToGetAddress,
             .failedToScanQr,
             .noUnsignedTransactionFound,
             .tapSignerSetupFailed,
             .tapSignerInvalidAuth,
             .tapSignerDeriveFailed,
             .general,
             .invalidFormat,
             .loading:
            .dismiss
        }
    }
}

private struct WalletAlertActions: View {
    let alert: AppAlertState
    let app: AppManager

    var body: some View {
        switch alert {
        case let .duplicateWallet(walletId):
            DuplicateWalletActions(walletId: walletId, app: app)

        case let .hotWalletKeyMissing(walletId):
            HotWalletKeyMissingActions(walletId: walletId, app: app)

        case .confirmWatchOnly:
            ConfirmWatchOnlyActions(app: app)

        case let .walletDatabaseCorrupted(walletId, error):
            CorruptedWalletActions(walletId: walletId, databaseError: error, app: app)

        default:
            EmptyView()
        }
    }
}

private struct DuplicateWalletActions: View {
    let walletId: WalletId
    let app: AppManager

    var body: some View {
        Button("OK") {
            app.alertState = .none
            app.isSidebarVisible = false
            try? app.selectWalletOrThrow(walletId)
        }
    }
}

private struct HotWalletKeyMissingActions: View {
    let walletId: WalletId
    let app: AppManager

    var body: some View {
        if CloudBackupManager.shared.isCloudBackupEnabled {
            Button("Open Cloud Backup") {
                app.alertState = .none
                app.loadAndReset(to: .settings(.cloudBackup))
            }
        }

        Button("Import 12 Words") {
            app.alertState = .none
            app.loadAndReset(to: .newWallet(.hotWallet(.import(.twelve, .manual))))
        }

        Button("Import 24 Words") {
            app.alertState = .none
            app.loadAndReset(to: .newWallet(.hotWallet(.import(.twentyFour, .manual))))
        }

        Button("Use with Hardware Wallet") {
            useAsHardwareWallet()
        }

        Button("Use as Watch Only", role: .cancel) {
            DispatchQueue.main.async { app.alertState = .init(.confirmWatchOnly) }
        }
    }

    private func useAsHardwareWallet() {
        Task { @MainActor in
            do {
                let manager = try await app.ensureWalletManagerLoaded(id: walletId)
                try await manager.setWalletType(.cold)
                app.alertState = .none
            } catch {
                Log.error("Failed to set wallet type to cold: \(error)")
                app.alertState = .init(
                    .general(
                        title: "Error",
                        message: error.localizedDescription
                    )
                )
            }
        }
    }
}

private struct ConfirmWatchOnlyActions: View {
    let app: AppManager

    var body: some View {
        Button("I Understand", role: .destructive) {
            app.alertState = .none
        }
    }
}

private struct CorruptedWalletActions: View {
    let walletId: WalletId
    let databaseError: String
    let app: AppManager

    private var retry: CorruptedWalletDeletionRetry? {
        app.corruptedWalletDeletionRetry.flatMap { retry in
            retry.walletId == walletId ? retry : nil
        }
    }

    var body: some View {
        Button(retry == nil ? "Delete Wallet" : "Retry", role: .destructive) {
            app.alertState = .none
            if let retry {
                app.retryCorruptedWalletDeletion(retry)
            } else {
                app.deleteCorruptedWallet(id: walletId, databaseError: databaseError)
            }
        }

        Button("Cancel", role: .cancel) {
            if let retry {
                app.cancelCorruptedWalletDeletion(retry)
            } else {
                app.alertState = .none
                app.trySelectLatestOrNewWallet()
            }
        }
    }
}

private struct AddressAlertActions: View {
    let alert: AppAlertState
    let app: AppManager

    var body: some View {
        switch alert {
        case let .addressWrongNetwork(address, _, _), let .noWalletSelected(address):
            CopyAddressActions(address: address, app: app)

        case let .foundAddress(address, amount):
            FoundAddressActions(address: address, amount: amount, app: app)

        case .noCameraPermission:
            CameraPermissionActions(app: app)

        default:
            EmptyView()
        }
    }
}

private struct CopyAddressActions: View {
    let address: Address
    let app: AppManager

    var body: some View {
        Button("Copy Address") {
            UIPasteboard.general.string = String(address)
        }

        Button("Cancel") {
            app.alertState = .none
        }
    }
}

private struct FoundAddressActions: View {
    let address: Address
    let amount: Amount?
    let app: AppManager

    var body: some View {
        Button("Copy Address") {
            UIPasteboard.general.string = String(address)
        }

        if let walletId = Database().globalConfig().selectedWallet() {
            Button("Send To Address") {
                let route = RouteFactory().sendSetAmount(
                    id: walletId,
                    address: address,
                    amount: amount
                )
                app.pushRoute(route)
                app.alertState = .none
            }
        }

        Button("Cancel") {
            app.alertState = .none
        }
    }
}

private struct CameraPermissionActions: View {
    let app: AppManager

    var body: some View {
        Button("OK") {
            app.alertState = .none
            let url = URL(string: UIApplication.openSettingsURLString)!
            UIApplication.shared.open(url)
        }
    }
}

private struct WatchOnlyAlertActions: View {
    let alert: AppAlertState
    let app: AppManager

    var body: some View {
        switch alert {
        case .cantSendOnWatchOnlyWallet:
            WatchOnlyImportChoiceActions(app: app)

        case .watchOnlyImportHardware:
            WatchOnlyHardwareActions(app: app)

        case .watchOnlyImportWords:
            WatchOnlyWordsActions(app: app)

        default:
            EmptyView()
        }
    }
}

private struct WatchOnlyImportChoiceActions: View {
    let app: AppManager

    var body: some View {
        Button("Import Hardware Wallet") {
            DispatchQueue.main.async { app.alertState = .init(.watchOnlyImportHardware) }
        }

        Button("Import Words") {
            DispatchQueue.main.async { app.alertState = .init(.watchOnlyImportWords) }
        }

        Button("Cancel", role: .cancel) {
            app.alertState = .none
        }
    }
}

private struct WatchOnlyHardwareActions: View {
    let app: AppManager

    var body: some View {
        Button("QR Code") {
            app.alertState = .none
            app.pushRoute(.newWallet(.coldWallet(.qrCode)))
        }

        Button("NFC") {
            app.alertState = .none
            app.nfcReader.scan()
        }

        Button("Paste") {
            importPastedWallet()
        }

        Button("Cancel", role: .cancel) {
            app.alertState = .none
        }
    }

    private func importPastedWallet() {
        app.alertState = .none
        let text = UIPasteboard.general.string ?? ""
        guard !text.isEmpty else { return }

        do {
            let wallet = try Wallet.newFromXpub(xpub: text)
            try app.selectWalletOrThrow(wallet.id())
            app.resetRoute(to: .selectedWallet(wallet.id()))
        } catch {
            DispatchQueue.main.async {
                app.alertState = .init(
                    .errorImportingHardwareWallet(message: error.localizedDescription)
                )
            }
        }
    }
}

private struct WatchOnlyWordsActions: View {
    let app: AppManager

    var body: some View {
        Button("Scan QR") {
            app.alertState = .none
            app.pushRoute(.newWallet(.hotWallet(.import(.twentyFour, .qr))))
        }

        Button("NFC") {
            app.alertState = .none
            app.pushRoute(.newWallet(.hotWallet(.import(.twentyFour, .nfc))))
        }

        Button("12 Words") {
            app.alertState = .none
            app.pushRoute(.newWallet(.hotWallet(.import(.twelve, .manual))))
        }

        Button("24 Words") {
            app.alertState = .none
            app.pushRoute(.newWallet(.hotWallet(.import(.twentyFour, .manual))))
        }

        Button("Cancel", role: .cancel) {
            app.alertState = .none
        }
    }
}

private struct TapSignerAlertActions: View {
    let alert: AppAlertState
    let app: AppManager

    var body: some View {
        switch alert {
        case let .uninitializedTapSigner(tapSigner):
            UninitializedTapSignerActions(tapSigner: tapSigner, app: app)

        case let .tapSignerWalletFound(walletId):
            TapSignerWalletFoundActions(walletId: walletId, app: app)

        case let .initializedTapSigner(tapSigner):
            InitializedTapSignerActions(tapSigner: tapSigner, app: app)

        case let .tapSignerNoBackup(tapSigner):
            TapSignerNoBackupActions(tapSigner: tapSigner, app: app)

        case let .tapSignerWrongPin(tapSigner, action):
            TapSignerWrongPinActions(tapSigner: tapSigner, action: action, app: app)

        default:
            EmptyView()
        }
    }
}

private struct UninitializedTapSignerActions: View {
    let tapSigner: TapSigner
    let app: AppManager

    var body: some View {
        Button("Yes") {
            app.isSidebarVisible = false
            app.sheetState = .init(.tapSigner(TapSignerRoute.initSelect(tapSigner)))
        }

        Button("Cancel", role: .cancel) {
            app.alertState = .none
        }
    }
}

private struct TapSignerWalletFoundActions: View {
    let walletId: WalletId
    let app: AppManager

    var body: some View {
        Button("Yes") {
            app.selectWallet(walletId)
        }

        Button("Cancel", role: .cancel) {
            app.alertState = .none
        }
    }
}

private struct InitializedTapSignerActions: View {
    let tapSigner: TapSigner
    let app: AppManager

    var body: some View {
        Button("Yes") {
            app.sheetState = .init(
                .tapSigner(
                    .enterPin(tapSigner: tapSigner, action: .derive)
                )
            )
        }

        Button("Cancel", role: .cancel) {
            app.alertState = .none
        }
    }
}

private struct TapSignerNoBackupActions: View {
    let tapSigner: TapSigner
    let app: AppManager

    var body: some View {
        Button("Yes") {
            print("TODO: go to backup screen \(tapSigner)}")
        }

        Button("Cancel", role: .cancel) {
            app.alertState = .none
        }
    }
}

private struct TapSignerWrongPinActions: View {
    let tapSigner: TapSigner
    let action: AfterPinAction
    let app: AppManager

    var body: some View {
        Button("Try Again") {
            app.sheetState = .init(.tapSigner(.enterPin(tapSigner: tapSigner, action: action)))
        }

        Button("Cancel", role: .cancel) {
            app.alertState = .none
        }
    }
}

private struct DismissAlertActions: View {
    let app: AppManager

    var body: some View {
        Button("OK") {
            app.alertState = .none
        }
    }
}
