import SwiftUI

/// One button of a root prompt: the action Rust offers for it and the intent it sends
///
/// A prompt draws a choice only when the screen has an entry for its action, disabled with Rust's
/// reason when the action can't run now, so its buttons follow admission like every other control
struct CloudBackupPromptChoice {
    let title: String
    let action: CloudBackupScreenAction
    let intent: CloudBackupManagerAction
    var role: ButtonRole?
    /// The element the choice is found by, when another choice in the prompt sends the same action
    var element: CloudBackupUiElement?

    var identifier: String {
        (element ?? .action(action: action)).identifier
    }

    /// The choices of the existing-backup prompt, in the order they are drawn
    static let existingBackup = [
        CloudBackupPromptChoice(
            title: "Create New Backup",
            action: .acceptEnablePromptCreateNew,
            intent: .acceptEnablePrompt(.createNew),
            role: .destructive
        ),
        CloudBackupPromptChoice(
            title: "Try Existing Passkey",
            action: .acceptEnablePromptUseExisting,
            intent: .acceptEnablePrompt(.useExisting)
        ),
    ]

    /// The choices of a passkey-choice prompt, in the order they are drawn
    ///
    /// Both repair choices answer the Repair Passkey entry: they differ only in whether the passkey
    /// request may offer an existing passkey, which admission doesn't decide on
    static func passkeyChoice(
        _ intent: CloudBackupPasskeyChoiceIntent,
        hint: CloudBackupPasskeyHint?
    ) -> [CloudBackupPromptChoice] {
        let existingTitle = hint.map { "Use Existing Passkey (\($0.nameSuffix))" } ?? "Use Existing Passkey"
        let useExisting = CloudBackupPromptChoice(
            title: existingTitle,
            action: .acceptEnablePromptUseExisting,
            intent: .acceptEnablePrompt(.useExisting)
        )

        switch intent {
        case .enable:
            return [
                useExisting,
                CloudBackupPromptChoice(
                    title: "Start a New Backup",
                    action: .acceptEnablePromptCreateNew,
                    intent: .acceptEnablePrompt(.createNew)
                ),
            ]
        case .enableExistingPasskeyOnly:
            return [useExisting]
        case .repairPasskey:
            return [
                CloudBackupPromptChoice(title: existingTitle, action: .repairPasskey, intent: .repairPasskey),
                CloudBackupPromptChoice(
                    title: "Create New Passkey",
                    action: .repairPasskey,
                    intent: .repairPasskeyNoDiscovery,
                    element: .promptPasskeyChoiceCreateNewPasskey
                ),
            ]
        }
    }
}

/// A prompt choice with the entry Rust sends for it now
struct CloudBackupOfferedPromptChoice {
    let choice: CloudBackupPromptChoice
    let availability: CloudBackupActionAvailability
}

extension [CloudBackupPromptChoice] {
    /// The choices Rust sends an entry for on `screen`, each with its availability
    func offered(on screen: CloudBackupScreen) -> [CloudBackupOfferedPromptChoice] {
        compactMap { choice in
            screen.actions.availability(action: choice.action).map {
                CloudBackupOfferedPromptChoice(choice: choice, availability: $0)
            }
        }
    }
}

extension [CloudBackupOfferedPromptChoice] {
    /// The prompt's message, followed by why a choice is disabled and the outcome of the last choice
    ///
    /// An alert has no room under its buttons, so the reasons and the outcome go in its message
    func message(_ base: String, on screen: CloudBackupScreen) -> String {
        var parts = [base]
        for reason in compactMap(\.availability.unavailableReason?.caption) where !parts.contains(reason) {
            parts.append(reason)
        }

        if let lastOutcome = screen.lastOutcome, case let .action(action) = lastOutcome.target,
           contains(where: { $0.choice.action == action }),
           !parts.contains(lastOutcome.outcome.message)
        {
            parts.append(lastOutcome.outcome.message)
        }

        return parts.joined(separator: "\n\n")
    }
}

/// The buttons of a prompt alert: each offered choice, then its Cancel
///
/// Cancel is the prompt's always-present escape and sends a dismiss intent admission always
/// accepts, so it has no entry and is never disabled
struct CloudBackupPromptChoiceButtons: View {
    @Environment(\.cloudBackupControlRecorder) private var recorder

    let choices: [CloudBackupOfferedPromptChoice]
    let choose: (CloudBackupPromptChoice) -> Void
    let cancel: () -> Void

    var body: some View {
        ForEach(choices, id: \.choice.identifier) { offered in
            Button(offered.choice.title, role: offered.choice.role) {
                choose(offered.choice)
            }
            .disabled(!offered.availability.isAvailable)
            .accessibilityIdentifier(offered.choice.identifier)
            .onAppear {
                recorder?.record(offered.choice.action, enabled: offered.availability.isAvailable)
            }
        }

        Button("Cancel", role: .cancel, action: cancel)
    }
}
