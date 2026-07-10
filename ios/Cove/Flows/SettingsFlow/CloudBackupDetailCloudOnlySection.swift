import SwiftUI

private extension CloudOnlyOperation {
    var operatingRecordId: String? {
        if case let .operating(recordId) = self { return recordId }
        return nil
    }
}

enum CloudBackupRestoreAllIntent: Equatable {
    case start
    case retry
}

struct CloudBackupRestoreAllActionPresentation: Equatable {
    let title: String
    let intent: CloudBackupRestoreAllIntent
}

struct CloudBackupRestoreAllProgressPresentation: Equatable {
    let completed: UInt32
    let total: UInt32
    let title: String
    let detail: String
    let accessibilityValue: String
    let canCancel: Bool
}

enum CloudBackupRestoreAllPresentation: Equatable {
    case hidden
    case disabled(title: String)
    case action(CloudBackupRestoreAllActionPresentation)
    case running(CloudBackupRestoreAllProgressPresentation)
}

func cloudBackupRestoreAllPresentation(
    state: CloudBackupRestoreAllState
) -> CloudBackupRestoreAllPresentation {
    switch state {
    case .notShown:
        return .hidden
    case let .startDisabled(walletCount):
        return .disabled(title: "Restore All (\(walletCount))")
    case let .startAvailable(walletCount):
        return .action(CloudBackupRestoreAllActionPresentation(
            title: "Restore All (\(walletCount))",
            intent: .start
        ))
    case let .retryDisabled(walletCount):
        return .disabled(title: "Retry Remaining (\(walletCount))")
    case let .retryAvailable(walletCount):
        return .action(CloudBackupRestoreAllActionPresentation(
            title: "Retry Remaining (\(walletCount))",
            intent: .retry
        ))
    case let .running(completed, total, currentWalletName, cancellationRequested):
        let title = currentWalletName.map { "Restoring \($0)" } ?? "Preparing wallet restores"
        let detail = "Completed \(completed) of \(total)"
        let cancellation = cancellationRequested
            ? "Cancel requested. The current wallet will finish first."
            : ""

        return .running(CloudBackupRestoreAllProgressPresentation(
            completed: completed,
            total: total,
            title: title,
            detail: detail,
            accessibilityValue: [title, detail, cancellation]
                .filter { !$0.isEmpty }
                .joined(separator: ", "),
            canCancel: !cancellationRequested
        ))
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
            CloudOnlyRestoreAllControl(manager: manager)

            CloudOnlyWalletRows(
                wallets: wallets,
                operatingRecordId: manager.cloudOnlyOperation.operatingRecordId,
                isOperating: isOperating || manager.restoreAllState.isRunning
                    || !manager.isDetailInventoryComplete,
                onSelectWallet: onSelectWallet,
                onRetryWallet: { item in
                    manager.dispatch(action: .restoreCloudWallet(item.recordId))
                }
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

private extension CloudBackupRestoreAllState {
    var isRunning: Bool {
        guard case .running = self else { return false }
        return true
    }
}

private struct CloudOnlyRestoreAllControl: View {
    let manager: CloudBackupManager

    private var presentation: CloudBackupRestoreAllPresentation {
        cloudBackupRestoreAllPresentation(state: manager.restoreAllState)
    }

    var body: some View {
        switch presentation {
        case .hidden:
            EmptyView()

        case let .disabled(title):
            Label(title, systemImage: "icloud.and.arrow.down")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                .accessibilityLabel("\(title), unavailable while Cloud Backup is checking")

        case let .action(action):
            Button {
                switch action.intent {
                case .start:
                    manager.startRestoreAll()
                case .retry:
                    manager.retryRestoreAllRemaining()
                }
            } label: {
                Label(
                    action.title,
                    systemImage: action.intent == .start
                        ? "square.stack.3d.up"
                        : "arrow.clockwise"
                )
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            }
            .accessibilityHint("Restores eligible wallets sequentially to this device")

        case let .running(progress):
            VStack(alignment: .leading, spacing: 10) {
                VStack(alignment: .leading, spacing: 10) {
                    ProgressView(
                        value: Double(progress.completed),
                        total: Double(max(progress.total, 1))
                    )
                    .accessibilityHidden(true)

                    Text(progress.title)
                        .font(.body.weight(.medium))
                        .fixedSize(horizontal: false, vertical: true)

                    Text(progress.detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("Restore All progress")
                .accessibilityValue(progress.accessibilityValue)
                .accessibilityAddTraits(.updatesFrequently)

                if progress.canCancel {
                    Button("Cancel") {
                        manager.cancelRestoreAll()
                    }
                    .frame(minHeight: 44)
                    .accessibilityHint("Stops after the current wallet finishes")
                } else {
                    Label(
                        "Cancel requested. Finishing the current wallet...",
                        systemImage: "hourglass"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(minHeight: 44)
                }
            }
            .padding(.vertical, 4)
        }
    }
}

private struct CloudOnlyWalletRows: View {
    let wallets: [CloudBackupWalletItem]
    let operatingRecordId: String?
    let isOperating: Bool
    let onSelectWallet: (CloudBackupWalletItem) -> Void
    let onRetryWallet: (CloudBackupWalletItem) -> Void

    var body: some View {
        ForEach(wallets, id: \.recordId) { item in
            VStack(alignment: .leading, spacing: 8) {
                Button {
                    onSelectWallet(item)
                } label: {
                    HStack {
                        if operatingRecordId == item.recordId {
                            ProgressView()
                                .padding(.trailing, 8)
                        }
                        WalletItemRow(
                            item: item,
                            accessibilityAction: item.syncStatus == .unsupportedVersion
                                ? "Restore requires a newer version of Cove; delete is available"
                                : "Restore or delete from iCloud"
                        )
                    }
                }
                .buttonStyle(.plain)
                .foregroundStyle(.primary)
                .disabled(isOperating)

                if item.restoreFailure != nil {
                    Button {
                        onRetryWallet(item)
                    } label: {
                        Label("Restore", systemImage: "arrow.clockwise")
                            .frame(minHeight: 44)
                    }
                    .buttonStyle(.borderless)
                    .disabled(isOperating)
                    .accessibilityLabel("Retry restoring \(item.name)")
                }
            }
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
                        guard manager.isDetailInventoryComplete else { return }

                        if item.syncStatus == .unsupportedVersion {
                            unsupportedRestoreWallet = item
                            return
                        }

                        manager.dispatch(action: .restoreCloudWallet(item.recordId))
                    }
                    .disabled(!manager.isDetailInventoryComplete)

                    Button("Delete from iCloud", role: .destructive) {
                        guard manager.isDetailInventoryComplete else { return }

                        walletToDelete = item
                    }
                    .disabled(!manager.isDetailInventoryComplete)
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
                        guard manager.isDetailInventoryComplete else { return }

                        manager.dispatch(action: .deleteCloudWallet(item.recordId))
                    }
                    .disabled(!manager.isDetailInventoryComplete)
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
