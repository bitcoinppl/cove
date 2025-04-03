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
    @Binding var exportingBackup: ExportingBackup?

    // confirmation dialogs
    @Binding var isExportingLabels: Bool
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
        isExportingLabels = true
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
                    exportingBackup = ExportingBackup(tapSigner: t, backup: backup)
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

            if case let .tapSigner(t) = metadata.hardwareMetadata {
                ChangePinButton(t)
                DownloadBackupButton(t)
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
            exportingBackup: Binding.constant(nil),
            isExportingLabels: Binding.constant(false),
            isImportingLabels: Binding.constant(false)
        )
        .environment(AppManager.shared)
    }
}
