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

    private var app: AppManager {
        context.app
    }

    var body: some View {
        switch alert {
        case let .duplicateWallet(walletId: walletId):
            Button("OK") {
                app.alertState = .none
                app.isSidebarVisible = false
                try? app.selectWalletOrThrow(walletId)
            }

        case let .hotWalletKeyMissing(walletId: walletId):
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
                do {
                    try app.ensureWalletManager(id: walletId).rust.setWalletType(walletType: .cold)
                    app.alertState = .none
                } catch {
                    Log.error("Failed to set wallet type to cold: \(error)")
                    DispatchQueue.main.async {
                        app.alertState = .init(
                            .general(
                                title: "Error",
                                message: error.localizedDescription
                            )
                        )
                    }
                }
            }

            Button("Use as Watch Only", role: .cancel) {
                DispatchQueue.main.async { app.alertState = .init(.confirmWatchOnly) }
            }

        case .confirmWatchOnly:
            Button("I Understand", role: .destructive) {
                app.alertState = .none
            }

        case let .addressWrongNetwork(address, _, _):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                app.alertState = .none
            }

        case let .noWalletSelected(address):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            Button("Cancel") {
                app.alertState = .none
            }

        case let .foundAddress(address: address, amount: amount):
            Button("Copy Address") {
                UIPasteboard.general.string = String(address)
            }

            if let id = Database().globalConfig().selectedWallet() {
                Button("Send To Address") {
                    let route = RouteFactory().sendSetAmount(
                        id: id,
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

        case .noCameraPermission:
            Button("OK") {
                app.alertState = .none
                let url = URL(string: UIApplication.openSettingsURLString)!
                UIApplication.shared.open(url)
            }

        case let .uninitializedTapSigner(tapSigner):
            Button("Yes") {
                app.isSidebarVisible = false
                app.sheetState = .init(.tapSigner(TapSignerRoute.initSelect(tapSigner)))
            }

            Button("Cancel", role: .cancel) {
                app.alertState = .none
            }

        case let .tapSignerWalletFound(walletId):
            Button("Yes") { app.selectWallet(walletId) }
            Button("Cancel", role: .cancel) { app.alertState = .none }

        case let .initializedTapSigner(tapSigner):
            Button("Yes") {
                app.sheetState = .init(
                    .tapSigner(
                        .enterPin(tapSigner: tapSigner, action: .derive)
                    )
                )
            }
            Button("Cancel", role: .cancel) { app.alertState = .none }

        case let .tapSignerNoBackup(tapSigner):
            Button("Yes") {
                print("TODO: go to backup screen \(tapSigner)}")
            }
            Button("Cancel", role: .cancel) { app.alertState = .none }

        case let .tapSignerWrongPin(tapSigner, action):
            Button("Try Again") {
                app.sheetState = .init(.tapSigner(.enterPin(tapSigner: tapSigner, action: action)))
            }
            Button("Cancel", role: .cancel) { app.alertState = .none }

        case .cantSendOnWatchOnlyWallet:
            Button("Import Hardware Wallet") {
                DispatchQueue.main.async { app.alertState = .init(.watchOnlyImportHardware) }
            }
            Button("Import Words") {
                DispatchQueue.main.async { app.alertState = .init(.watchOnlyImportWords) }
            }
            Button("Cancel", role: .cancel) {
                app.alertState = .none
            }

        case .watchOnlyImportHardware:
            Button("QR Code") {
                app.alertState = .none
                app.pushRoute(.newWallet(.coldWallet(.qrCode)))
            }
            Button("NFC") {
                app.alertState = .none
                app.nfcReader.scan()
            }
            Button("Paste") {
                app.alertState = .none
                let text = UIPasteboard.general.string ?? ""
                if text.isEmpty { return }

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
            Button("Cancel", role: .cancel) {
                app.alertState = .none
            }

        case .watchOnlyImportWords:
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

        case let .walletDatabaseCorrupted(walletId, _):
            Button("Delete Wallet", role: .destructive) {
                app.alertState = .none
                app.rust.deleteCorruptedWallet(id: walletId)
            }
            Button("Cancel", role: .cancel) {
                app.alertState = .none
                app.trySelectLatestOrNewWallet()
            }

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
            Button("OK") {
                app.alertState = .none
            }
        }
    }
}
