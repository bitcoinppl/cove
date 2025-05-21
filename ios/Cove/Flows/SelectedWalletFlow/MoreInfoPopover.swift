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
    @Binding var exporting: SelctedWalletScreenExporterView.Exporting?
    @Binding var isImportingLabels: Bool

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
        exporting = .labels
    }

    func exportTransactions() {
        // if task isnt cancelled in 0.5 seconds show alert about waiting
        let alertTask = Task {
            do {
                try await Task.sleep(for: .seconds(0.5))
                app.alertState = .init(
                    .general(
                        title: "Exporting, please wait...",
                        message: "Creating a transaction export file. If this is the first time it might take a while"
                    )
                )
            }
        }

        Task {
            do {
                let csv = try await manager.rust.createTransactionsWithFiatExport()
                alertTask.cancel()

                if app.alertState != .none {
                    await MainActor.run { app.alertState = .none }
                    try? await Task.sleep(for: .seconds(0.5))
                }

                await MainActor.run { exporting = .transactions(csv) }
            } catch {
                app.alertState = .init(
                    .general(
                        title: "Ooops something went wrong!",
                        message: "Unable to export transactions \(error.localizedDescription)"
                    )
                )
            }
        }
    }

    var defaultFileName: String {
        labelManager.exportDefaultFileName(name: metadata.name)
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
        let action = {
            if let backup = app.getTapSignerBackup(t) {
                return {
                    Log.debug("Downloading backup...")
                    exporting = .backup(ExportingBackup(tapSigner: t, backup: backup))
                }
            }

            let route = TapSignerRoute.enterPin(tapSigner: t, action: .backup)
            return { app.sheetState = .init(.tapSigner(route)) }
        }()

        Button(action: action) {
            Label("Download Backup", systemImage: "square.and.arrow.down")
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
            exporting: Binding.constant(nil),
            isImportingLabels: Binding.constant(false)
        )
        .environment(AppManager.shared)
    }
}
