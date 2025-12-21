//
//  MoreInfoPopover.swift
//  Cove
//
//  Created by Praveen Perera on 2/11/25.
//

import SwiftUI

struct MoreInfoPopover: View {
    @Environment(AppManager.self) private var app

    // args
    let manager: WalletManager
    @Binding var isImportingLabels: Bool

    // bindings
    @Binding var showExportLabelsConfirmation: Bool

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
        Task {
            do {
                let result = try await manager.rust.exportTransactionsCsv()
                ShareSheet.present(data: result.content, filename: result.filename) { success in
                    if !success {
                        Log.warn("Transaction Export Failed: cancelled or failed")
                    }
                }
            } catch {
                app.alertState = .init(.general(
                    title: "Transaction Export Failed",
                    message: "Unable to export transactions: \(error.localizedDescription)"
                ))
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
