//
//  TransactionDetailsLabelView.swift
//  Cove
//
//  Created by Praveen Perera on 2/13/25.
//

import SwiftUI

struct TransactionDetailsLabelView: View {
    @Environment(AppManager.self) private var app

    @State var details: TransactionDetails
    let manager: WalletManager

    @State var isEditing = false
    @State var editingLabel = ""

    @FocusState private var isFocused: Bool

    var labelManager: LabelManager {
        manager.rust.labelManager()
    }

    var txId: TxId {
        details.txId()
    }

    var label: String? {
        if !editingLabel.isEmpty {
            return editingLabel
        }

        return details.transactionLabel()
    }

    func setEditing() {
        withAnimation {
            editingLabel = label ?? ""
            isEditing = true
            isFocused = true
        }
    }

    /// get updated details and full transaction list that has the new label
    func updateDetailsAndTxns() {
        Task { await manager.rust.getTransactions() }

        Task {
            do {
                let details = try await manager.rust.transactionDetails(txId: txId)
                await MainActor.run {
                    self.details = details
                    manager.transactionDetails[details.txId()] = details
                }
            } catch {
                await manager.rust.getTransactions()
                Log.error("Error getting updated label: \(error)")
            }
        }
    }

    func saveLabel() {
        do {
            try labelManager.insertOrUpdateLabelsForTxn(
                details: details,
                label: editingLabel,
                origin: manager.walletMetadata.origin
            )

            updateDetailsAndTxns()

            withAnimation {
                isEditing = false
                isFocused = false
            }
        } catch {
            Log.error("Unable to save label: \(error)")
        }
    }

    func deleteLabel() {
        do {
            try labelManager.deleteLabelsForTxn(txId: txId)
            isEditing = false
            editingLabel = ""

            updateDetailsAndTxns()
        } catch {
            Log.error("Unable to delete label: \(error)")
        }
    }

    func TxnLabel(_ label: String) -> some View {
        Menu {
            Button("Edit", systemImage: "square.and.pencil", action: setEditing)
            Button("Delete", systemImage: "trash", role: .destructive, action: deleteLabel)
        } label: {
            Image(systemName: "tag.circle.fill")
                .foregroundStyle(.primary)

            Text(label)
                .foregroundStyle(.secondary)
        }
        .foregroundStyle(.secondary)
    }

    var AddLabel: some View {
        Button(action: setEditing) {
            HStack {
                Image(systemName: "plus.circle.fill")
                    .symbolRenderingMode(.multicolor)

                Text("Add label")
                    .foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(.secondary)
    }

    var EditingLabel: some View {
        HStack {
            Spacer()

            Image(systemName: "square.and.pencil")

            TextField(label ?? "Add label", text: $editingLabel)
                .foregroundStyle(.secondary)
                .fixedSize()
                .focused($isFocused)
                .offset(y: 1.2)

            Spacer()
        }
    }

    var body: some View {
        Group {
            if isEditing {
                EditingLabel
            } else {
                if let label {
                    TxnLabel(label)
                } else {
                    AddLabel
                }
            }
        }
        .font(.footnote)
        .onChange(of: isFocused, initial: false) { old, new in
            // lost focused
            if old, !new { saveLabel() }
        }
    }
}

#Preview("No Label") {
    AsyncPreview {
        TransactionDetailsLabelView(
            details: TransactionDetails.previewNewConfirmed(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("With Label") {
    AsyncPreview {
        TransactionDetailsLabelView(
            details: TransactionDetails.previewNewWithLabel(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("Editing Label") {
    AsyncPreview {
        TransactionDetailsLabelView(
            details: TransactionDetails.previewNewWithLabel(label: "Car payment"),
            manager: WalletManager(preview: "preview_only"),
            isEditing: true
        )
        .environment(AppManager.shared)
    }
}
