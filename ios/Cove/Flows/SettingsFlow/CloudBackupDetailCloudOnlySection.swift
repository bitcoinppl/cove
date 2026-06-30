import SwiftUI

private extension CloudOnlyOperation {
    var operatingRecordId: String? {
        if case let .operating(recordId) = self { return recordId }
        return nil
    }
}

struct CloudOnlySection: View {
    let manager: CloudBackupManager
    @State private var selectedWallet: CloudBackupWalletItem?
    @State private var walletToDelete: CloudBackupWalletItem?
    @State private var unsupportedRestoreWallet: CloudBackupWalletItem?

    private var isOperating: Bool {
        manager.cloudOnlyOperation.operatingRecordId != nil
    }

    var body: some View {
        Section(header: Text("Not on This Device")) {
            CloudOnlySectionContent(
                manager: manager,
                isOperating: isOperating,
                onSelectWallet: { selectedWallet = $0 }
            )
        }
        .modifier(
            CloudOnlyActionDialogs(
                manager: manager,
                selectedWallet: $selectedWallet,
                walletToDelete: $walletToDelete,
                unsupportedRestoreWallet: $unsupportedRestoreWallet
            )
        )
    }
}

private struct CloudOnlySectionContent: View {
    let manager: CloudBackupManager
    let isOperating: Bool
    let onSelectWallet: (CloudBackupWalletItem) -> Void

    var body: some View {
        switch manager.cloudOnly {
        case .notFetched, .loading:
            HStack {
                ProgressView()
                    .padding(.trailing, 8)
                Text("Loading...")
            }
            .foregroundStyle(.secondary)
            .task {
                manager.dispatch(action: .fetchCloudOnly)
            }

        case let .loaded(wallets):
            CloudOnlyWalletRows(
                wallets: wallets,
                operatingRecordId: manager.cloudOnlyOperation.operatingRecordId,
                isOperating: isOperating,
                onSelectWallet: onSelectWallet
            )

            if case let .failed(error) = manager.cloudOnlyOperation {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(Color.statusError)
            } else if case let .warning(message: message, error: _) = manager.cloudOnlyOperation {
                Text(message)
                    .font(.caption)
                    .foregroundStyle(Color.statusWarning)
            }

        case let .failed(error):
            Text(error)
                .font(.caption)
                .foregroundStyle(Color.statusError)
        }
    }
}

private struct CloudOnlyWalletRows: View {
    let wallets: [CloudBackupWalletItem]
    let operatingRecordId: String?
    let isOperating: Bool
    let onSelectWallet: (CloudBackupWalletItem) -> Void

    var body: some View {
        ForEach(wallets, id: \.recordId) { item in
            Button {
                onSelectWallet(item)
            } label: {
                HStack {
                    if operatingRecordId == item.recordId {
                        ProgressView()
                            .padding(.trailing, 8)
                    }
                    WalletItemRow(item: item)
                }
            }
            .foregroundStyle(.primary)
            .disabled(isOperating)
        }
    }
}

private struct CloudOnlyActionDialogs: ViewModifier {
    let manager: CloudBackupManager
    @Binding var selectedWallet: CloudBackupWalletItem?
    @Binding var walletToDelete: CloudBackupWalletItem?
    @Binding var unsupportedRestoreWallet: CloudBackupWalletItem?

    func body(content: Content) -> some View {
        content
            .confirmationDialog(
                selectedWallet?.name ?? "Wallet",
                isPresented: Binding(
                    get: { selectedWallet != nil },
                    set: { if !$0 { selectedWallet = nil } }
                ),
                titleVisibility: .visible
            ) {
                if let item = selectedWallet {
                    Button("Restore to This Device") {
                        if item.syncStatus == .unsupportedVersion {
                            unsupportedRestoreWallet = item
                            return
                        }

                        manager.dispatch(action: .restoreCloudWallet(item.recordId))
                    }
                    Button("Delete from iCloud", role: .destructive) {
                        walletToDelete = item
                    }
                }
                Button("Cancel", role: .cancel) {}
            }
            .alert(
                "Delete \(walletToDelete?.name ?? "wallet")?",
                isPresented: Binding(
                    get: { walletToDelete != nil },
                    set: { if !$0 { walletToDelete = nil } }
                )
            ) {
                if let item = walletToDelete {
                    Button("Delete Forever", role: .destructive) {
                        manager.dispatch(action: .deleteCloudWallet(item.recordId))
                    }
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This wallet backup will be permanently removed from iCloud")
            }
            .alert(
                "Can't Restore \(unsupportedRestoreWallet?.name ?? "Wallet")",
                isPresented: Binding(
                    get: { unsupportedRestoreWallet != nil },
                    set: { if !$0 { unsupportedRestoreWallet = nil } }
                )
            ) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(
                    "This backup uses a newer version of Cove and can't be restored on this device yet"
                )
            }
    }
}
