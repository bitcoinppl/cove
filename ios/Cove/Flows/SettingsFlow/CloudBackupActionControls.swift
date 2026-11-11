import SwiftUI

/// Records which Cloud Backup controls a rendered screen drew and whether each was enabled
///
/// The platform contract test renders every Rust screen fixture with a recorder in the
/// environment and checks the record against the fixture's actions; production views have none
final class CloudBackupControlRecorder {
    struct RowControl: Hashable {
        let recordId: String
        let action: CloudBackupRowActionKind
    }

    private(set) var actions: [CloudBackupScreenAction: Bool] = [:]
    private(set) var rowActions: [RowControl: Bool] = [:]

    func record(_ action: CloudBackupScreenAction, enabled: Bool) {
        actions[action] = (actions[action] ?? false) || enabled
    }

    func record(row recordId: String, _ action: CloudBackupRowActionKind, enabled: Bool) {
        let control = RowControl(recordId: recordId, action: action)
        rowActions[control] = (rowActions[control] ?? false) || enabled
    }
}

/// The last outcome Rust reported and how to dismiss it, so each control draws the outcome that
/// answers it
struct CloudBackupOutcomeContext {
    let lastOutcome: CloudBackupLastOutcome?
    let dismiss: () -> Void

    func outcome(for action: CloudBackupScreenAction) -> CloudBackupOutcome? {
        guard let lastOutcome, case .action(action) = lastOutcome.target else { return nil }
        return lastOutcome.outcome
    }

    func outcome(forRow recordId: RecordId, _ action: CloudBackupRowActionKind) -> CloudBackupOutcome? {
        guard let lastOutcome, case let .row(target) = lastOutcome.target else { return nil }
        guard target == CloudBackupRowTarget(recordId: recordId, control: action.control()) else {
            return nil
        }

        return lastOutcome.outcome
    }
}

extension EnvironmentValues {
    @Entry var cloudBackupControlRecorder: CloudBackupControlRecorder?
    @Entry var cloudBackupOutcomes: CloudBackupOutcomeContext?
}

/// One control for a global Cloud Backup action
///
/// Draws nothing when the screen has no entry for the action, and draws it disabled with Rust's
/// reason underneath when the action can't run now
struct CloudBackupActionControl<Label: View>: View {
    @Environment(\.cloudBackupControlRecorder) private var recorder
    @Environment(\.cloudBackupOutcomes) private var outcomes
    @Environment(\.isEnabled) private var isEnabled

    let action: CloudBackupScreenAction
    let actions: CloudBackupActions
    var role: ButtonRole?
    var showsCaption = true
    /// The element a control that isn't the action's own control is found by, such as the heal
    /// notice's Turn Off
    var element: CloudBackupUiElement?
    let perform: () -> Void
    @ViewBuilder let label: () -> Label

    private var availability: CloudBackupActionAvailability? {
        actions.availability(action: action)
    }

    /// one container, so modifiers such as a confirmation dialog attach once rather than to the
    /// button and its caption separately
    var body: some View {
        if let availability {
            VStack(alignment: .leading, spacing: 4) {
                Button(role: role, action: perform, label: label)
                    .disabled(!availability.isAvailable)
                    .accessibilityIdentifier((element ?? .action(action: action)).identifier)
                    .onAppear {
                        recorder?.record(action, enabled: isEnabled && availability.isAvailable)
                    }

                if showsCaption, let reason = availability.unavailableReason {
                    CloudBackupUnavailableCaption(action: action, reason: reason)
                }

                if let outcomes, let outcome = outcomes.outcome(for: action) {
                    CloudBackupOutcomeNotice(outcome: outcome, dismiss: outcomes.dismiss)
                }
            }
        }
    }
}

/// One control for an action on a listed row
///
/// Draws no caption: the row's section shows the reason once when its rows share it
struct CloudBackupRowActionControl<Label: View>: View {
    @Environment(\.cloudBackupControlRecorder) private var recorder
    @Environment(\.isEnabled) private var isEnabled

    let recordId: RecordId
    let rowAction: CloudBackupRowAction
    var role: ButtonRole?
    let perform: () -> Void
    @ViewBuilder let label: () -> Label

    var body: some View {
        Button(role: role, action: perform, label: label)
            .disabled(!rowAction.availability.isAvailable)
            .accessibilityIdentifier(
                CloudBackupUiElement.rowAction(
                    target: CloudBackupRowTarget(recordId: recordId, control: rowAction.action.control())
                ).identifier
            )
            .onAppear {
                recorder?.record(
                    row: recordId,
                    rowAction.action,
                    enabled: isEnabled && rowAction.availability.isAvailable
                )
            }
    }
}

/// Why a control is disabled, under the control
struct CloudBackupUnavailableCaption: View {
    let action: CloudBackupScreenAction
    let reason: CloudBackupRejection

    var body: some View {
        Text(reason.caption)
            .font(.caption)
            .uiTestTruncationProbe()
            .foregroundStyle(.secondary)
            .accessibilityIdentifier(CloudBackupUiElement.actionUnavailable(action: action).identifier)
    }
}

/// The result of the last intent that did not simply succeed, next to the control it answers
struct CloudBackupOutcomeNotice: View {
    let outcome: CloudBackupOutcome
    let dismiss: () -> Void

    @AccessibilityFocusState private var isFocused: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            ProbedLabel(outcome.message, systemImage: "exclamationmark.circle")
                .font(.caption)
                .foregroundStyle(Color.statusWarning)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityFocused($isFocused)
                .accessibilityIdentifier(CloudBackupUiElement.lastOutcome.identifier)

            Spacer()

            Button("Dismiss", action: dismiss)
                .font(.caption)
                .buttonStyle(.borderless)
                .accessibilityIdentifier(CloudBackupUiElement.lastOutcomeDismiss.identifier)
        }
        .onAppear { isFocused = true }
    }
}

extension CloudBackupUiElement {
    /// The identifier both platforms give this element; Rust owns the mapping, listed in
    /// `docs/cloud_backup.md`
    var identifier: String {
        accessibilityIdentifier()
    }
}

extension CloudBackupScreenAction {
    /// The identifier of the control that sends this action
    var accessibilityIdentifier: String {
        CloudBackupUiElement.action(action: self).identifier
    }
}
