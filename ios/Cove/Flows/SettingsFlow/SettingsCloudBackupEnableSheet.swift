import SwiftUI

struct SettingsCloudBackupEnableSheet: View {
    @State private var manager = CloudBackupManager.shared
    @State private var ignoreNextPromptDismiss = false

    let onComplete: () -> Void
    let onDismiss: () -> Void

    private var message: String? {
        if manager.isUnsupportedPasskeyProvider {
            "This passkey provider did not confirm PRF support for Cloud Backup. Try Apple Passwords (iCloud Keychain) or another supported provider such as 1Password"
        } else {
            manager.lifecycleFailureMessage
        }
    }

    private var isBusy: Bool {
        if case .awaitingSavedPasskeyConfirmation(.manual) = manager.enableFlow {
            return false
        }

        if isAwaitingEnablePrompt(manager.rootPrompt) {
            return false
        }

        return manager.isLifecycleEnabling
    }

    private var showingPasskeyChoice: Binding<Bool> {
        Binding(
            get: { isEnablePasskeyChoice(manager.rootPrompt) },
            set: { isPresented in
                guard !isPresented else { return }
                handlePromptDismiss()
            }
        )
    }

    private var showingExistingBackupPrompt: Binding<Bool> {
        Binding(
            get: {
                if case .existingBackupFound = manager.rootPrompt { return true }
                return false
            },
            set: { isPresented in
                guard !isPresented else { return }
                handlePromptDismiss()
            }
        )
    }

    private var passkeyChoiceIntent: CloudBackupPasskeyChoiceIntent? {
        guard case let .passkeyChoice(intent) = manager.rootPrompt else { return nil }

        return intent
    }

    private var passkeyChoicePresentation: CloudBackupPasskeyChoicePresentation? {
        guard let passkeyChoiceIntent else { return nil }

        return CloudBackupPasskeyChoicePresentation(intent: passkeyChoiceIntent)
    }

    private var existingBackupPasskeyHint: CloudBackupPasskeyHint? {
        guard case let .existingBackupFound(_, passkeyHint) = manager.rootPrompt else {
            return nil
        }

        return passkeyHint
    }

    private func isEnablePasskeyChoice(_ rootPrompt: CloudBackupRootPrompt) -> Bool {
        guard case let .passkeyChoice(intent) = rootPrompt else { return false }

        switch intent {
        case .enable, .enableExistingPasskeyOnly:
            return true
        case .repairPasskey:
            return false
        }
    }

    private func isAwaitingEnablePrompt(_ rootPrompt: CloudBackupRootPrompt) -> Bool {
        if case .existingBackupFound = rootPrompt { return true }
        return isEnablePasskeyChoice(rootPrompt)
    }

    private func beginEnableChoice() {
        guard !isBusy, !isAwaitingEnablePrompt(manager.rootPrompt) else { return }
        manager.dispatch(action: .promptEnablePasskeyChoice(.init(
            savedPasskeyConfirmation: .manual,
            verificationSource: .settings
        )))
    }

    private func dispatchPromptAction(_ action: CloudBackupManagerAction) {
        ignoreNextPromptDismiss = true
        manager.dispatch(action: action)
    }

    private func handlePromptDismiss() {
        if ignoreNextPromptDismiss {
            ignoreNextPromptDismiss = false
            return
        }

        switch manager.rootPrompt {
        case .existingBackupFound:
            manager.dispatch(action: .discardPendingEnableCloudBackup)
        case .passkeyChoice(.enable), .passkeyChoice(.enableExistingPasskeyOnly):
            manager.dispatch(action: .dismissPasskeyChoicePrompt)
        case .none, .missingPasskeyReminder, .passkeyChoice(.repairPasskey), .verification:
            break
        }
    }

    private func completeIfReady(_ completion: TaggedItem<CloudBackupEnableContext>?) {
        guard let completion, completion.item.verificationSource == .settings else { return }

        manager.consumeEnableCompletion(completion)
        onComplete()
    }

    private func existingPasskeyButtonTitle(for hint: CloudBackupPasskeyHint?) -> String {
        guard let hint else { return "Use Existing Passkey" }
        return "Use Existing Passkey (\(hint.nameSuffix))"
    }

    private var existingBackupMessage: String {
        guard let existingBackupPasskeyHint else {
            return "Creating a new Cloud Backup will not include wallets from your previous backup. If you still have access to the passkey for that backup, use the existing passkey instead."
        }

        return "Creating a new Cloud Backup will not include wallets from your previous backup. If you still have access to the passkey named Cove Cloud Backup (\(existingBackupPasskeyHint.nameSuffix)), use that passkey instead."
    }

    var body: some View {
        ZStack {
            if case .awaitingSavedPasskeyConfirmation(.manual) = manager.enableFlow {
                CloudBackupEnableConfirmationView(
                    onContinue: {
                        manager.dispatch(action: .confirmSavedPasskey)
                    },
                    onCancel: {
                        manager.dispatch(action: .discardPendingEnableCloudBackup)
                        onDismiss()
                    }
                )
            } else {
                CloudBackupEnableOnboardingView(
                    onEnable: beginEnableChoice,
                    onCancel: onDismiss,
                    message: message,
                    isBusy: isBusy
                )
            }

            if isBusy {
                CloudBackupEnableBusyOverlay(
                    enableFlow: manager.enableFlow,
                    verificationPresentation: manager.verificationPresentation
                )
            }
        }
        .onChange(of: manager.enableCompletion, initial: true) { _, completion in
            completeIfReady(completion)
        }
        .onChange(of: manager.rootPrompt, initial: true) { _, rootPrompt in
            if !isAwaitingEnablePrompt(rootPrompt) {
                ignoreNextPromptDismiss = false
            }
        }
        .alert(
            "Passkey Options",
            isPresented: showingPasskeyChoice
        ) {
            Button(existingPasskeyButtonTitle(for: passkeyChoicePresentation?.passkeyHint)) {
                dispatchPromptAction(.acceptEnablePrompt(.useExisting))
            }
            if let secondaryActionTitle = passkeyChoicePresentation?.secondaryActionTitle {
                Button(secondaryActionTitle) {
                    dispatchPromptAction(.acceptEnablePrompt(.createNew))
                }
            }
            Button("Cancel", role: .cancel) {
                dispatchPromptAction(.dismissPasskeyChoicePrompt)
            }
        } message: {
            Text(passkeyChoicePresentation?.message ?? "Choose how to continue with Cloud Backup.")
        }
        .alert(
            "Existing Cloud Backup Found",
            isPresented: showingExistingBackupPrompt
        ) {
            Button("Create New Backup", role: .destructive) {
                dispatchPromptAction(.acceptEnablePrompt(.createNew))
            }
            Button("Try Existing Passkey") {
                dispatchPromptAction(.acceptEnablePrompt(.useExisting))
            }
            Button("Cancel", role: .cancel) {
                dispatchPromptAction(.discardPendingEnableCloudBackup)
            }
        } message: {
            Text(existingBackupMessage)
        }
    }
}
