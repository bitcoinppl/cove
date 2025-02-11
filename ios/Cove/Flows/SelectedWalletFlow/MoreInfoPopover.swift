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
    let labelManager: LabelManager

    // confirmation dialogs
    @Binding var showingExportOptions: Bool
    @Binding var showingImportOptions: Bool

    private var hasLabels: Bool {
        labelManager.hasLabels()
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    func importLabels() {
        showingImportOptions = true
    }

    func exportLabels() {
        showingExportOptions = true
    }

    var defaultFileName: String {
        labelManager.exportDefaultFileName(name: metadata.name)
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
                    Label("Export Labels", systemImage: "square.and.arrow.")
                }
            }
        }
    }
}

#Preview {
    AsyncPreview {
        MoreInfoPopover(
            manager: WalletManager(preview: "preview_only"),
            labelManager: LabelManager(id: WalletManager(preview: "preview_only").walletMetadata.id),
            showingExportOptions: Binding.constant(false),
            showingImportOptions: Binding.constant(false)
        )
        .environment(AppManager.shared)
    }
}
