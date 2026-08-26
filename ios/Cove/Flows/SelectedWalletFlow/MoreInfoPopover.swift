//
//  MoreInfoPopover.swift
//  Cove
//
//  Created by Praveen Perera on 2/11/25.
//

import SwiftUI

struct MoreInfoPopover: View {
    @Environment(AppManager.self) private var app

    let manager: WalletManager
    let importLabels: () -> Void
    let exportLabels: () -> Void
    let exportXpub: () -> Void

    private var hasLabels: Bool {
        (try? manager.labelManager().hasLabels()) ?? false
    }

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    @State private var tapSignerBackup: Data? = nil
    @State private var tapSignerBackupError: Error? = nil

    var body: some View {
        VStack {
            MoreInfoBasicActions(
                hasLabels: hasLabels,
                hasTransactions: manager.hasTransactions,
                scanNfc: app.nfcReader.scan,
                importLabels: importLabels,
                exportLabels: exportLabels,
                exportTransactions: exportTransactions,
                exportXpub: exportXpub
            )

            if case let .tapSigner(tapSigner) = metadata.hardwareMetadata {
                TapSignerMoreInfoActions(
                    tapSigner: tapSigner,
                    backup: tapSignerBackup,
                    backupError: tapSignerBackupError
                )
            }

            MoreInfoWalletActions(
                hasTransactions: manager.hasTransactions,
                manageUtxos: manageUtxos,
                openWalletSettings: openWalletSettings
            )
        }
        .tint(Color(uiColor: .label))
        .onAppear(perform: loadTapSignerBackup)
    }

    private func exportTransactions() {
        Task {
            do {
                let result = try await manager.exportTransactionsCsv()
                ShareSheet.presentFromMenu(data: result.content, filename: result.filename)
            } catch {
                app.alertState = .init(.general(
                    title: "Transaction Export Failed",
                    message: "Unable to export transactions: \(error.localizedDescription)"
                ))
            }
        }
    }

    private func loadTapSignerBackup() {
        guard case let .tapSigner(tapSigner) = metadata.hardwareMetadata else { return }

        do {
            tapSignerBackup = try app.getTapSignerBackup(tapSigner)
        } catch {
            tapSignerBackupError = error
        }
    }

    private func manageUtxos() {
        app.pushRoute(.coinControl(.list(metadata.id)))
    }

    private func openWalletSettings() {
        app.pushRoute(.settings(.wallet(id: metadata.id, route: .main)))
    }
}

private struct MoreInfoBasicActions: View {
    let hasLabels: Bool
    let hasTransactions: Bool
    let scanNfc: () -> Void
    let importLabels: () -> Void
    let exportLabels: () -> Void
    let exportTransactions: () -> Void
    let exportXpub: () -> Void

    var body: some View {
        VStack {
            Button(action: scanNfc) {
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

            if hasTransactions {
                Button(action: exportTransactions) {
                    Label("Export Transactions", systemImage: "arrow.up.arrow.down")
                }
            }

            Button(action: exportXpub) {
                Label("Export Xpub", systemImage: "key.horizontal")
            }
        }
    }
}

private struct TapSignerMoreInfoActions: View {
    @Environment(AppManager.self) private var app

    let tapSigner: TapSigner
    let backup: Data?
    let backupError: Error?

    var body: some View {
        VStack {
            Button(action: changePin) {
                Label("Change PIN", systemImage: "key")
            }

            TapSignerBackupButton(
                tapSigner: tapSigner,
                backup: backup,
                backupError: backupError
            )
        }
    }

    private func changePin() {
        let route = TapSignerRoute.enterPin(tapSigner: tapSigner, action: .change)
        app.sheetState = .init(.tapSigner(route))
    }
}

private struct TapSignerBackupButton: View {
    @Environment(AppManager.self) private var app

    let tapSigner: TapSigner
    let backup: Data?
    let backupError: Error?

    var body: some View {
        if let backup {
            Button(action: { download(backup) }) {
                Label("Download Backup", systemImage: "square.and.arrow.down")
            }
        } else if let backupError {
            Button(action: { showError(backupError) }) {
                Label("Download Backup", systemImage: "square.and.arrow.down")
            }
        } else {
            Button(action: requestBackup) {
                Label("Download Backup", systemImage: "square.and.arrow.down")
            }
        }
    }

    private func download(_ backup: Data) {
        let content = hexEncode(bytes: backup)
        let prefix = tapSigner.identFileNamePrefix()
        let filename = "\(prefix)_backup.txt"

        ShareSheet.presentFromMenu(data: content, filename: filename)
    }

    private func showError(_ error: Error) {
        app.alertState = .init(.general(
            title: "Backup Error",
            message: "Failed to retrieve backup: \(error.localizedDescription)"
        ))
    }

    private func requestBackup() {
        let route = TapSignerRoute.enterPin(tapSigner: tapSigner, action: .backup)
        app.sheetState = .init(.tapSigner(route))
    }
}

private struct MoreInfoWalletActions: View {
    let hasTransactions: Bool
    let manageUtxos: () -> Void
    let openWalletSettings: () -> Void

    var body: some View {
        VStack {
            if hasTransactions {
                Button(action: manageUtxos) {
                    Label("Manage UTXOs", systemImage: "circlebadge.2")
                }
            }

            Button(action: openWalletSettings) {
                Label("Wallet Settings", systemImage: "gear")
            }
        }
    }
}

#Preview {
    AsyncPreview {
        MoreInfoPopover(
            manager: WalletManager(preview: .only),
            importLabels: {},
            exportLabels: {},
            exportXpub: {}
        )
        .environment(AppManager.shared)
    }
}
