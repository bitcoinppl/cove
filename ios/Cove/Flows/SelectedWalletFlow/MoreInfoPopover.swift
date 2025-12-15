//
//  MoreInfoPopover.swift
//  Cove
//
//  Created by Praveen Perera on 2/11/25.
//

import MijickPopups
import SwiftUI

private class LoadingState {
    var popupWasShown = false
    var popupShownAt: Date?
}

private let loadingPopupDelay: Duration = .milliseconds(250)
private let minimumPopupDisplayTime: TimeInterval = 0.4

struct MoreInfoPopover: View {
    @Environment(AppManager.self) private var app

    // args
    let manager: WalletManager
    @Binding var isImportingLabels: Bool

    // bindings
    @Binding var showExportLabelsConfirmation: Bool

    // state
    @State private var showLoadingTask: Task<Void, Never>?
    @State private var exportTask: Task<Void, Never>?

    private var hasLabels: Bool {
        labelManager.hasLabels()
    }

    var labelManager: LabelManager {
        manager.rust.labelManager()
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    func importLabels() {
        isImportingLabels = true
    }

    func exportLabels() {
        showExportLabelsConfirmation = true
    }

    func exportTransactions() {
        performExport(
            operation: {
                let csv = try await manager.rust.createTransactionsWithFiatExport()
                let filename = "\(metadata.name.lowercased())_transactions.csv"
                return (csv, filename)
            },
            errorTitle: "Transaction Export Failed",
            errorPrefix: "Unable to export transactions"
        )
    }

    private func performExport(
        operation: @escaping () async throws -> (data: String, filename: String),
        errorTitle: String,
        errorPrefix: String
    ) {
        let loadingState = LoadingState()

        // start delayed loading task
        showLoadingTask = Task { @MainActor in
            try? await Task.sleep(for: loadingPopupDelay)
            if Task.isCancelled { return }

            loadingState.popupWasShown = true
            loadingState.popupShownAt = Date.now
            await MiddlePopup(state: .loading).present()
        }

        // start export operation
        exportTask = Task {
            do {
                let (data, filename) = try await operation()

                // cancel loading if not shown yet
                showLoadingTask?.cancel()

                // check if cancelled before continuing
                if Task.isCancelled { return }

                // if popup was shown, ensure minimum display time
                if loadingState.popupWasShown, let shownAt = loadingState.popupShownAt {
                    let elapsed = Date.now.timeIntervalSince(shownAt)
                    let remaining = max(0, minimumPopupDisplayTime - elapsed)

                    if remaining > 0 {
                        try? await Task.sleep(for: .seconds(remaining))
                    }

                    await dismissAllPopups()
                }

                // check if cancelled before showing share sheet
                if Task.isCancelled { return }

                // show ShareSheet
                await MainActor.run {
                    ShareSheet.present(data: data, filename: filename) { success in
                        if !success {
                            Log.warn("\(errorTitle): cancelled or failed")
                        }
                    }
                }
            } catch {
                showLoadingTask?.cancel()

                // don't show error if cancelled
                if Task.isCancelled { return }

                await MainActor.run {
                    if loadingState.popupWasShown {
                        Task { await dismissAllPopups() }
                    }

                    app.alertState = .init(.general(
                        title: errorTitle,
                        message: "\(errorPrefix): \(error.localizedDescription)"
                    ))
                }
            }
        }
    }

    @ViewBuilder
    func ChangePinButton(_ t: TapSigner) -> some View {
        let route = TapSignerRoute.enterPin(tapSigner: t, action: .change)
        let action = { app.sheetState = .init(.tapSigner(route)) }
        Button(action: action) {
            Label("Change PIN", systemImage: "key")
        }
    }

    @ViewBuilder
    func DownloadBackupButton(_ t: TapSigner) -> some View {
        if let backup = app.getTapSignerBackup(t) {
            let content = hexEncode(bytes: backup)
            let prefix = t.identFileNamePrefix()
            let filename = "\(prefix)_backup.txt"

            ShareLink(
                item: BackupExport(content: content, filename: filename),
                preview: SharePreview(filename)
            ) {
                Label("Download Backup", systemImage: "square.and.arrow.down")
            }
        } else {
            Button(action: {
                let route = TapSignerRoute.enterPin(tapSigner: t, action: .backup)
                app.sheetState = .init(.tapSigner(route))
            }) {
                Label("Download Backup", systemImage: "square.and.arrow.down")
            }
        }
    }

    var body: some View {
        VStack {
            Button(action: app.nfcReader.scan) {
                Label("Scan NFC", systemImage: "wave.3.right")
            }

            Button(action: importLabels) {
                Label("Import Labels", systemImage: "square.and.arrow.down")
            }

            if hasLabels {
                Button(action: exportLabels) {
                    Label("Export Labels", systemImage: "square.and.arrow.up")
                }
            }

            if manager.hasTransactions {
                Button(action: exportTransactions) {
                    Label("Export Transactions", systemImage: "arrow.up.arrow.down")
                }
            }

            if case let .tapSigner(t) = metadata.hardwareMetadata {
                ChangePinButton(t)
                DownloadBackupButton(t)
            }

            if manager.hasTransactions {
                Button(action: {
                    app.pushRoute(.coinControl(.list(metadata.id)))
                }) {
                    Label("Manage UTXOs", systemImage: "circlebadge.2")
                }
            }

            // wallet settings last button
            Button(action: { app.pushRoute(.settings(.wallet(id: metadata.id, route: .main))) }) {
                Label("Wallet Settings", systemImage: "gear")
            }
        }
        .tint(.primary)
        .onDisappear {
            showLoadingTask?.cancel()
            exportTask?.cancel()
        }
    }
}

#Preview {
    AsyncPreview {
        MoreInfoPopover(
            manager: WalletManager(preview: "preview_only"),
            isImportingLabels: Binding.constant(false),
            showExportLabelsConfirmation: Binding.constant(false)
        )
        .environment(AppManager.shared)
    }
}
