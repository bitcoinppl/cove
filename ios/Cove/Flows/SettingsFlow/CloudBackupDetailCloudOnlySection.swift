import SwiftUI

struct CloudBackupRestoreAllProgressPresentation: Equatable {
    let completed: UInt32
    let total: UInt32
    let title: String
    let detail: String
    let accessibilityValue: String
    let cancellationRequested: Bool
}

/// What the Restore All control shows; whether it can be tapped is the screen's actions
enum CloudBackupRestoreAllPresentation: Equatable {
    case hidden
    case start(title: String)
    case retry(title: String)
    case running(CloudBackupRestoreAllProgressPresentation)
}

func cloudBackupRestoreAllPresentation(
    state: CloudBackupRestoreAllState
) -> CloudBackupRestoreAllPresentation {
    switch state {
    case .notShown:
        return .hidden
    case let .startDisabled(walletCount), let .startAvailable(walletCount):
        return .start(title: "Restore All (\(walletCount))")
    case let .retryDisabled(walletCount), let .retryAvailable(walletCount):
        return .retry(title: "Retry Remaining (\(walletCount))")
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
            cancellationRequested: cancellationRequested
        ))
    }
}

/// The reason every disabled row action in a section shares, shown once on the section rather
/// than on each row
func cloudBackupSharedRowReason(_ rows: [CloudBackupListedRow]) -> CloudBackupRejection? {
    let reasons = rows.flatMap(\.actions).compactMap(\.availability.unavailableReason)
    guard let first = reasons.first, reasons.allSatisfy({ $0 == first }) else { return nil }

    return first
}

/// Backups of wallets that aren't on this device, and backups Cove can't match to one
struct CloudBackupNotOnDeviceSectionView: View {
    let section: CloudBackupNotOnDeviceSection
    let actions: CloudBackupActions
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var sharedReason: CloudBackupRejection? {
        cloudBackupSharedRowReason(section.rows)
    }

    var body: some View {
        Section {
            CloudOnlyRestoreAllControl(
                state: section.restoreAll,
                actions: actions,
                manager: manager
            )

            CloudBackupListedRows(
                rows: section.rows,
                sharedReason: sharedReason,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )

            CloudOnlyOperationMessage(operation: section.operation)
        } header: {
            Text("Not on This Device").uiTestTruncationProbe()
        } footer: {
            CloudBackupSharedRowReasonFooter(reason: sharedReason)
        }
    }
}

/// More backups of wallets this device already holds, kept out of Not on This Device
struct CloudBackupExtraCopiesSectionView: View {
    let section: CloudBackupExtraCopiesSection
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    private var sharedReason: CloudBackupRejection? {
        cloudBackupSharedRowReason(section.rows)
    }

    var body: some View {
        Section {
            CloudBackupListedRows(
                rows: section.rows,
                sharedReason: sharedReason,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )
        } header: {
            Text("Extra Copies in iCloud").uiTestTruncationProbe()
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                Text("These wallets are already on this device under another backup. You can delete these extra copies from iCloud.")
                    .uiTestTruncationProbe()
                CloudBackupSharedRowReasonFooter(reason: sharedReason)
            }
        }
    }
}

private struct CloudBackupSharedRowReasonFooter: View {
    let reason: CloudBackupRejection?

    var body: some View {
        if let reason {
            Text(reason.caption)
                .uiTestTruncationProbe()
                .accessibilityIdentifier(CloudBackupUiElement.rowsUnavailable.identifier)
        }
    }
}

private struct CloudOnlyOperationMessage: View {
    let operation: CloudOnlyOperation

    var body: some View {
        switch operation {
        case let .failed(error):
            Text(error)
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(Color.statusError)
        case let .warning(message, _):
            Text(message)
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(Color.statusWarning)
        case .idle, .operating:
            EmptyView()
        }
    }
}

private struct CloudOnlyRestoreAllControl: View {
    let state: CloudBackupRestoreAllState
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        switch cloudBackupRestoreAllPresentation(state: state) {
        case .hidden:
            EmptyView()

        case let .start(title):
            CloudBackupActionControl(
                action: .startRestoreAll,
                actions: actions,
                perform: { manager.dispatch(action: .startRestoreAll) }
            ) {
                ProbedLabel(title, systemImage: "square.stack.3d.up")
                    .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            }
            .accessibilityHint("Restores eligible wallets sequentially to this device")

        case let .retry(title):
            CloudBackupActionControl(
                action: .retryRestoreAllRemaining,
                actions: actions,
                perform: { manager.dispatch(action: .retryRestoreAllRemaining) }
            ) {
                ProbedLabel(title, systemImage: "arrow.clockwise")
                    .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            }
            .accessibilityHint("Restores eligible wallets sequentially to this device")

        case let .running(progress):
            CloudOnlyRestoreAllProgressControl(
                presentation: progress,
                actions: actions,
                manager: manager
            )
        }
    }
}

private struct CloudOnlyRestoreAllProgressControl: View {
    let presentation: CloudBackupRestoreAllProgressPresentation
    let actions: CloudBackupActions
    let manager: CloudBackupManager

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            CloudOnlyRestoreAllProgressDetails(presentation: presentation)

            if presentation.cancellationRequested {
                CloudOnlyRestoreAllCancellationPending()
            }

            CloudBackupActionControl(
                action: .cancelRestoreAll,
                actions: actions,
                perform: { manager.dispatch(action: .cancelRestoreAll) }
            ) {
                Text("Cancel").uiTestTruncationProbe()
            }
            .frame(minHeight: 44)
            .accessibilityHint("Stops after the current wallet finishes")
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
                .uiTestTruncationProbe()
                .fixedSize(horizontal: false, vertical: true)

            Text(presentation.detail)
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Restore All progress")
        .accessibilityValue(presentation.accessibilityValue)
        .accessibilityAddTraits(.updatesFrequently)
        .uiTestTruncationReport("cloudBackup.restoreAll.progress.truncation")
    }
}

private struct CloudOnlyRestoreAllCancellationPending: View {
    var body: some View {
        ProbedLabel(
            "Cancel requested. Finishing the current wallet...",
            systemImage: "hourglass"
        )
        .font(.caption)
        .foregroundStyle(.secondary)
        .fixedSize(horizontal: false, vertical: true)
        .frame(minHeight: 44)
    }
}

private struct CloudBackupListedRows: View {
    let rows: [CloudBackupListedRow]
    let sharedReason: CloudBackupRejection?
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    var body: some View {
        ForEach(rows, id: \.recordId) { row in
            CloudBackupListedRowView(
                row: row,
                sharedReason: sharedReason,
                manager: manager,
                presentationCoordinator: presentationCoordinator
            )
        }
    }
}

/// One listed backup with the actions Rust gave it
private struct CloudBackupListedRowView: View {
    @Environment(\.cloudBackupOutcomes) private var outcomes

    let row: CloudBackupListedRow
    let sharedReason: CloudBackupRejection?
    let manager: CloudBackupManager
    let presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>

    /// a reason the section already shows once is not repeated on each row
    private var rowReasons: [CloudBackupRejection] {
        let reasons = row.actions.compactMap(\.availability.unavailableReason)
        return reasons.filter { $0 != sharedReason }.uniqued()
    }

    private var rowOutcome: CloudBackupOutcome? {
        guard let outcomes else { return nil }
        return row.actions.lazy.compactMap { outcomes.outcome(forRow: row.recordId, $0.action) }.first
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            CloudBackupListedRowLabel(record: row.record)
                .uiTestTruncationReport("cloudBackup.wallet.\(row.recordId).truncation")

            HStack(spacing: 16) {
                ForEach(row.actions, id: \.action) { rowAction in
                    CloudBackupRowActionControl(
                        recordId: row.recordId,
                        rowAction: rowAction,
                        role: rowAction.isDelete ? .destructive : nil,
                        perform: { perform(rowAction.action) }
                    ) {
                        ProbedLabel(rowAction.action.title, systemImage: rowAction.action.systemImage)
                            .font(.subheadline)
                            .frame(minHeight: 44)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel("\(rowAction.action.title) \(rowName)")
                }
            }

            ForEach(rowReasons, id: \.self) { reason in
                Text(reason.caption)
                    .font(.caption)
                    .uiTestTruncationProbe()
                    .foregroundStyle(.secondary)
            }

            if let rowOutcome, let outcomes {
                CloudBackupOutcomeNotice(outcome: rowOutcome, dismiss: outcomes.dismiss)
            }
        }
    }

    private var rowName: String {
        row.record.wallet?.name ?? row.record.unnamedTitle
    }

    private func perform(_ action: CloudBackupRowActionKind) {
        switch action {
        case .restore:
            manager.dispatch(action: .restoreCloudWallet(row.recordId))
        case let .delete(_, consent):
            presentationCoordinator.present(.alert(.rowDelete(row.recordId, consent)))
        case .backUpFirst:
            manager.dispatch(action: .syncUnsynced)
        case .checkAgain:
            manager.dispatch(action: .refreshDetail)
        case .verify:
            manager.dispatch(action: .startVerification(.cloudBackupDetail))
        }
    }
}

private struct CloudBackupListedRowLabel: View {
    let record: CloudBackupListedRecord

    var body: some View {
        if let wallet = record.wallet {
            WalletItemRow(item: wallet)
        } else {
            ProbedLabel(record.unnamedTitle, systemImage: "exclamationmark.triangle")
                .foregroundStyle(Color.statusWarning)
        }

        if let caption = record.caption {
            Text(caption)
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

private extension CloudBackupRowAction {
    var isDelete: Bool {
        if case .delete = action { return true }
        return false
    }
}

private extension Array where Element: Hashable {
    func uniqued() -> [Element] {
        var seen = Set<Element>()
        return filter { seen.insert($0).inserted }
    }
}
