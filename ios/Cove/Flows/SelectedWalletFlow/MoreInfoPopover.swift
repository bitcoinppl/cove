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
    @Binding var showExportXpubConfirmation: Bool

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

    func exportXpub() {
        showExportXpubConfirmation = true
    }

    func exportTransactions() {
        Task {
            do {
                let result = try await manager.rust.exportTransactionsCsv()
                ShareSheet.presentFromMenu(data: result.content, filename: result.filename)
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

    @State private var tapSignerBackup: Data? = nil
    @State private var tapSignerBackupError: Error? = nil

    @ViewBuilder
    func DownloadBackupButton(_ t: TapSigner) -> some View {
        if let backup = tapSignerBackup {
            let content = hexEncode(bytes: backup)
            let prefix = t.identFileNamePrefix()
            let filename = "\(prefix)_backup.txt"

            Button(action: { ShareSheet.presentFromMenu(data: content, filename: filename) }) {
                Label("Download Backup", systemImage: "square.and.arrow.down")
            }
        } else if let backupError = tapSignerBackupError {
            Button(action: {
                app.alertState = .init(.general(
                    title: "Backup Error",
                    message: "Failed to retrieve backup: \(backupError.localizedDescription)"
                ))
            }) {
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

    func loadTapSignerBackup(_ t: TapSigner) {
        do {
            tapSignerBackup = try app.getTapSignerBackup(t)
        } catch {
            tapSignerBackupError = error
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

            Button(action: exportXpub) {
                Label("Export Xpub", systemImage: "key.horizontal")
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
        .tint(Color(uiColor: .label))
        .onAppear {
            if case let .tapSigner(t) = metadata.hardwareMetadata {
                loadTapSignerBackup(t)
            }
        }
    }
}

#Preview {
    AsyncPreview {
        MoreInfoPopover(
            manager: WalletManager(preview: "preview_only"),
            isImportingLabels: Binding.constant(false),
            showExportLabelsConfirmation: Binding.constant(false),
            showExportXpubConfirmation: Binding.constant(false)
        )
        .environment(AppManager.shared)
    }
}
