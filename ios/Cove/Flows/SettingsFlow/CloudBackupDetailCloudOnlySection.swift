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
    let wallets: [CloudBackupWalletItem]
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var isOperating: Bool {
        manager.cloudOnlyOperation.operatingRecordId != nil
    }

    var body: some View {
        CloudOnlyFormSection(
            wallets: wallets,
            manager: manager,
            isOperating: isOperating,
            presentationCoordinator: presentationCoordinator
        )
    }
}

private struct CloudOnlyFormSection: View {
    let wallets: [CloudBackupWalletItem]
    let manager: CloudBackupManager
    let isOperating: Bool
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        Section(header: Text("Not on This Device")) {
            CloudOnlySectionContent(
                wallets: wallets,
                manager: manager,
                isOperating: isOperating,
                presentationCoordinator: presentationCoordinator
            )
        }
    }
}

private struct CloudOnlySectionContent: View {
    let wallets: [CloudBackupWalletItem]
    let manager: CloudBackupManager
    let isOperating: Bool
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        CloudOnlyRestoreAllControl(manager: manager)

        CloudOnlyWalletRows(
            manager: manager,
            wallets: wallets,
            operatingRecordId: manager.cloudOnlyOperation.operatingRecordId,
            isOperating: isOperating || manager.restoreAllState.isRunning
                || !manager.isDetailInventoryReady,
            presentationCoordinator: presentationCoordinator,
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
            CloudOnlyRestoreAllDisabledControl(title: title)

        case let .action(action):
            CloudOnlyRestoreAllActionControl(
                presentation: action,
                onStart: manager.startRestoreAll,
                onRetry: manager.retryRestoreAllRemaining
            )

        case let .running(progress):
            CloudOnlyRestoreAllProgressControl(
                presentation: progress,
                onCancel: manager.cancelRestoreAll
            )
        }
    }
}

private struct CloudOnlyRestoreAllDisabledControl: View {
    let title: String

    var body: some View {
        Label(title, systemImage: "icloud.and.arrow.down")
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .accessibilityLabel("\(title), unavailable while Cloud Backup is checking")
    }
}

private struct CloudOnlyRestoreAllActionControl: View {
    let presentation: CloudBackupRestoreAllActionPresentation
    let onStart: () -> Void
    let onRetry: () -> Void

    private var systemImage: String {
        presentation.intent == .start
            ? "square.stack.3d.up"
            : "arrow.clockwise"
    }

    var body: some View {
        Button(action: performAction) {
            Label(presentation.title, systemImage: systemImage)
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        }
        .accessibilityHint("Restores eligible wallets sequentially to this device")
    }

    private func performAction() {
        switch presentation.intent {
        case .start:
            onStart()
        case .retry:
            onRetry()
        }
    }
}

private struct CloudOnlyRestoreAllProgressControl: View {
    let presentation: CloudBackupRestoreAllProgressPresentation
    let onCancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            CloudOnlyRestoreAllProgressDetails(presentation: presentation)

            if presentation.canCancel {
                Button("Cancel", action: onCancel)
                    .frame(minHeight: 44)
                    .accessibilityHint("Stops after the current wallet finishes")
            } else {
                CloudOnlyRestoreAllCancellationPending()
            }
        }
        .padding(.vertical, 4)
    }
}

private struct CloudOnlyRestoreAllProgressDetails: View {
    let presentation: CloudBackupRestoreAllProgressPresentation

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ProgressView(
                value: Double(presentation.completed),
                total: Double(max(presentation.total, 1))
            )
            .accessibilityHidden(true)

            Text(presentation.title)
                .font(.body.weight(.medium))
                .fixedSize(horizontal: false, vertical: true)

            Text(presentation.detail)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Restore All progress")
        .accessibilityValue(presentation.accessibilityValue)
        .accessibilityAddTraits(.updatesFrequently)
    }
}

private struct CloudOnlyRestoreAllCancellationPending: View {
    var body: some View {
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

private struct CloudOnlyWalletRows: View {
    let manager: CloudBackupManager
    let wallets: [CloudBackupWalletItem]
    let operatingRecordId: String?
    let isOperating: Bool
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>
    let onRetryWallet: (CloudBackupWalletItem) -> Void

    var body: some View {
        ForEach(wallets, id: \.recordId) { item in
            VStack(alignment: .leading, spacing: 8) {
                CloudOnlyWalletActionButton(
                    item: item,
                    isOperating: isOperating,
                    isCurrentOperation: operatingRecordId == item.recordId,
                    presentationCoordinator: presentationCoordinator
                )

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

private struct CloudOnlyWalletActionButton: View {
    let item: CloudBackupWalletItem
    let isOperating: Bool
    let isCurrentOperation: Bool
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        Button {
            presentationCoordinator.present(.dialog(.cloudOnlyWalletActions(item)))
        } label: {
            CloudOnlyWalletActionLabel(
                item: item,
                isCurrentOperation: isCurrentOperation
            )
        }
        .buttonStyle(.plain)
        .foregroundStyle(.primary)
        .disabled(isOperating)
    }
}

private struct CloudOnlyWalletActionLabel: View {
    let item: CloudBackupWalletItem
    let isCurrentOperation: Bool

    private var accessibilityAction: String {
        item.syncStatus == .unsupportedVersion
            ? "Restore requires a newer version of Cove; delete is available"
            : "Restore or delete from iCloud"
    }

    var body: some View {
        HStack {
            if isCurrentOperation {
                ProgressView()
                    .padding(.trailing, 8)
            }

            WalletItemRow(item: item, accessibilityAction: accessibilityAction)
        }
    }
}
