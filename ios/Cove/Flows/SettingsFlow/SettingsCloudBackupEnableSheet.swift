import SwiftUI
import UIKit

func cloudBackupPendingEnableSupportEmailURL(
    supportCode: String,
    appVersion: String
) -> URL? {
    var components = URLComponents()
    components.scheme = "mailto"
    components.path = "feedback@covebitcoinwallet.com"
    components.queryItems = [
        URLQueryItem(name: "subject", value: "Cove Cloud Backup recovery \(supportCode)"),
        URLQueryItem(
            name: "body",
            value: "Support code: \(supportCode)\nPlatform: iOS\nApp version: \(appVersion)"
        ),
    ]

    return components.url
}

struct SettingsCloudBackupEnableSheet: View {
    @State private var manager = CloudBackupManager.shared
    @State private var ignoreNextPromptDismiss = false

    let onComplete: () -> Void
    let onDismiss: () -> Void

    private var screen: CloudBackupScreen {
        manager.screen
    }

    private var message: String? {
        if case .unsupportedProvider = screen.configured?.passkeyCard {
            return "This passkey provider did not confirm PRF support for Cloud Backup. Try Apple Passwords (iCloud Keychain) or another supported provider such as 1Password"
        }

        return screen.enableSheetOutcome?.message
    }

    private var showingPasskeyChoice: Binding<Bool> {
        Binding(
            get: { isEnablePasskeyChoice(screen.rootPrompt) },
            set: { isPresented in
                guard !isPresented else { return }
                handlePromptDismiss()
            }
        )
    }

    private var showingExistingBackupPrompt: Binding<Bool> {
        Binding(
            get: {
                if case .existingBackupFound = screen.rootPrompt { return true }
                return false
            },
            set: { isPresented in
                guard !isPresented else { return }
                handlePromptDismiss()
            }
        )
    }

    private var passkeyChoiceIntent: CloudBackupPasskeyChoiceIntent? {
        guard case let .passkeyChoice(intent) = screen.rootPrompt else { return nil }

        return intent
    }

    private var passkeyChoicePresentation: CloudBackupPasskeyChoicePresentation? {
        guard let passkeyChoiceIntent else { return nil }

        return CloudBackupPasskeyChoicePresentation(intent: passkeyChoiceIntent)
    }

    private var existingBackupPasskeyHint: CloudBackupPasskeyHint? {
        guard case let .existingBackupFound(_, passkeyHint) = screen.rootPrompt else {
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

        switch screen.rootPrompt {
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

    private var existingBackupMessage: String {
        guard let existingBackupPasskeyHint else {
            return "Creating a new Cloud Backup will not include wallets from your previous backup. If you still have access to the passkey for that backup, use the existing passkey instead."
        }

        return "Creating a new Cloud Backup will not include wallets from your previous backup. If you still have access to the passkey named Cove Cloud Backup (\(existingBackupPasskeyHint.nameSuffix)), use that passkey instead."
    }

    /// The prompts' alerts are bound to the screen's root prompt, so a choice Rust rejects leaves
    /// the prompt it still projects, and the alert comes back with the rejection in it
    var body: some View {
        CloudBackupEnableRootContent(
            screen: screen,
            message: message,
            onRemoveIncompleteSetup: confirmPendingEnableCleanup,
            onConfirmSavedPasskey: confirmSavedPasskey,
            onCancelConfirmation: cancelSavedPasskeyConfirmation,
            onEnable: beginEnableChoice,
            onCancel: onDismiss
        )
        .modifier(CloudBackupPromptAlertModifier(
            title: "Existing Cloud Backup Found",
            isPresented: showingExistingBackupPrompt,
            message: existingBackupMessage,
            choices: CloudBackupPromptChoice.existingBackup,
            screen: screen,
            choose: choose,
            cancel: discardPendingEnable
        ))
        .modifier(CloudBackupPromptAlertModifier(
            title: "Passkey Options",
            isPresented: showingPasskeyChoice,
            message: passkeyChoicePresentation?.message ?? "Choose how to continue with Cloud Backup.",
            choices: passkeyChoicePresentation?.choices ?? [],
            screen: screen,
            choose: choose,
            cancel: dismissPasskeyChoice
        ))
        .onChange(of: manager.enableCompletion, initial: true) { _, completion in
            completeIfReady(completion)
        }
        .onChange(of: screen.rootPrompt, initial: true) { _, rootPrompt in
            if !isAwaitingEnablePrompt(rootPrompt) {
                ignoreNextPromptDismiss = false
            }
        }
    }

    private func choose(_ choice: CloudBackupPromptChoice) {
        dispatchPromptAction(choice.intent)
    }

    private func dismissPasskeyChoice() {
        dispatchPromptAction(.dismissPasskeyChoicePrompt)
    }

    private func discardPendingEnable() {
        dispatchPromptAction(.discardPendingEnableCloudBackup)
    }

    private func confirmPendingEnableCleanup() {
        manager.dispatch(action: .confirmPendingEnableCleanup)
    }

    private func confirmSavedPasskey() {
        manager.dispatch(action: .confirmSavedPasskey)
    }

    private func cancelSavedPasskeyConfirmation() {
        manager.dispatch(action: .discardPendingEnableCloudBackup)
        onDismiss()
    }
}

private struct CloudBackupEnableRootContent: View {
    let screen: CloudBackupScreen
    let message: String?
    let onRemoveIncompleteSetup: () -> Void
    let onConfirmSavedPasskey: () -> Void
    let onCancelConfirmation: () -> Void
    let onEnable: () -> Void
    let onCancel: () -> Void

    private var isWorking: Bool {
        screen.enableFlow?.isWorking ?? false
    }

    var body: some View {
        ZStack {
            CloudBackupEnableFlowContent(
                screen: screen,
                message: message,
                isBusy: isWorking,
                onRemoveIncompleteSetup: onRemoveIncompleteSetup,
                onConfirmSavedPasskey: onConfirmSavedPasskey,
                onCancelConfirmation: onCancelConfirmation,
                onEnable: onEnable,
                onCancel: onCancel
            )

            if isWorking {
                CloudBackupEnableBusyOverlay(enableFlow: screen.enableFlow)
            }
        }
    }
}

private struct CloudBackupEnableFlowContent: View {
    private let content: AnyView

    init(
        screen: CloudBackupScreen,
        message: String?,
        isBusy: Bool,
        onRemoveIncompleteSetup: @escaping () -> Void,
        onConfirmSavedPasskey: @escaping () -> Void,
        onCancelConfirmation: @escaping () -> Void,
        onEnable: @escaping () -> Void,
        onCancel: @escaping () -> Void
    ) {
        if let recovery = screen.pendingEnableRecovery {
            content = AnyView(CloudBackupPendingEnableRecoveryView(
                recovery: recovery,
                actions: screen.actions,
                onRemoveIncompleteSetup: onRemoveIncompleteSetup,
                onCancel: onCancel
            ))
        } else if case .awaitingSavedPasskeyConfirmation(.manual) = screen.enableFlow {
            content = AnyView(CloudBackupEnableConfirmationView(
                continueAvailability: screen.actions.availability(action: .confirmSavedPasskey),
                onContinue: onConfirmSavedPasskey,
                onCancel: onCancelConfirmation
            ))
        } else {
            content = AnyView(CloudBackupEnableOnboardingView(
                onEnable: onEnable,
                onCancel: onCancel,
                message: message,
                isBusy: isBusy,
                primaryAvailability: screen.actions.availability(action: .enable)
            ))
        }
    }

    var body: some View {
        content
    }
}

struct CloudBackupPendingEnableRecoveryView: View {
    @Environment(\.openURL) private var openURL

    let recovery: CloudBackupPendingEnableRecovery
    let actions: CloudBackupActions
    let onRemoveIncompleteSetup: () -> Void
    let onCancel: () -> Void

    private var isCleaning: Bool {
        recovery.cleanup == .cleaning
    }

    /// the explanation names cleanup only when this setup has local data Cove can remove
    private var canRemoveIncompleteSetup: Bool {
        recovery.cleanup != .supportOnly
    }

    var body: some View {
        VStack(spacing: 0) {
            CloudBackupEnableCancelButton(isBusy: isCleaning, onCancel: onCancel)

            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    CloudBackupRecoveryHeader()
                    CloudBackupRecoveryExplanation(
                        canRemoveIncompleteSetup: canRemoveIncompleteSetup
                    )
                    CloudBackupRecoverySupportCodeCard(
                        supportCode: recovery.supportCode,
                        copy: copySupportCode,
                        contactSupport: contactSupport
                    )
                    CloudBackupRecoveryActions(
                        isCleaning: isCleaning,
                        actions: actions,
                        removeIncompleteSetup: onRemoveIncompleteSetup
                    )
                }
                .padding()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(CloudBackupEnableBackground())
    }

    private func copySupportCode() {
        UIPasteboard.general.string = recovery.supportCode
    }

    private func contactSupport() {
        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "unknown"
        if let url = cloudBackupPendingEnableSupportEmailURL(
            supportCode: recovery.supportCode,
            appVersion: version
        ) {
            openURL(url)
        }
    }
}

private struct CloudBackupRecoveryHeader: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Image(systemName: "exclamationmark.icloud.fill")
                .font(.system(size: 42))
                .foregroundStyle(Color.statusWarning)
            Text("Cloud Backup Needs Recovery")
                .font(.largeTitle.weight(.semibold))
                .uiTestTruncationProbe()
                .foregroundStyle(.white)
            Text("Cloud Backup setup was interrupted and its local recovery records do not match.")
                .font(.footnote)
                .uiTestTruncationProbe()
                .foregroundStyle(.coveLightGray.opacity(0.75))
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct CloudBackupRecoveryExplanation: View {
    let canRemoveIncompleteSetup: Bool

    var body: some View {
        Text(
            canRemoveIncompleteSetup
                ? "Cove verified that the incomplete local setup can be removed without changing your active backup or cloud data."
                : "Contact support and include the code below. Don’t change Cloud Backup settings until the recovery state has been reviewed."
        )
        .font(.callout)
        .uiTestTruncationProbe()
        .foregroundStyle(.white.opacity(0.85))
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.duskBlue.opacity(0.5))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

private struct CloudBackupRecoverySupportCodeCard: View {
    let supportCode: String
    let copy: () -> Void
    let contactSupport: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Support code")
                .font(.caption)
                .uiTestTruncationProbe()
                .foregroundStyle(.coveLightGray)
            Text(supportCode)
                .font(.title3.monospaced().weight(.semibold))
                .uiTestTruncationProbe()
                .foregroundStyle(.white)
                .accessibilityIdentifier(CloudBackupUiElement.recoverySupportCode.identifier)
            CloudBackupRecoverySupportButtons(copy: copy, contactSupport: contactSupport)
                .buttonStyle(.bordered)
                .tint(.white)
        }
        .padding(16)
        .background(Color.duskBlue.opacity(0.5))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

/// Copy Code and Contact Support, side by side when both fit on one line and stacked otherwise
///
/// Side by side at accessibility sizes, each button had room for only part of "Support", which
/// iOS split with a hyphen
private struct CloudBackupRecoverySupportButtons: View {
    let copy: () -> Void
    let contactSupport: () -> Void

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack {
                copyButton
                Spacer()
                contactSupportButton
            }

            VStack(alignment: .leading) {
                copyButton
                contactSupportButton
            }
        }
    }

    private var copyButton: some View {
        Button(action: copy) {
            ProbedLabel("Copy Code", systemImage: "doc.on.doc")
        }
    }

    private var contactSupportButton: some View {
        Button(action: contactSupport) {
            ProbedLabel("Contact Support", systemImage: "envelope")
        }
    }
}

private struct CloudBackupRecoveryActions: View {
    let isCleaning: Bool
    let actions: CloudBackupActions
    let removeIncompleteSetup: () -> Void

    var body: some View {
        if isCleaning {
            CloudBackupRecoveryCleanupProgress()
        } else {
            CloudBackupRecoveryCleanupButton(
                actions: actions,
                removeIncompleteSetup: removeIncompleteSetup
            )
        }
    }
}

private struct CloudBackupRecoveryCleanupProgress: View {
    var body: some View {
        HStack {
            Spacer()
            ProgressView("Removing incomplete setup...")
                .tint(.white)
                .foregroundStyle(.white)
            Spacer()
        }
        .padding(.vertical)
    }
}

private struct CloudBackupRecoveryCleanupButton: View {
    let actions: CloudBackupActions
    let removeIncompleteSetup: () -> Void
    @State private var showConfirmation = false

    var body: some View {
        CloudBackupActionControl(
            action: .confirmPendingEnableCleanup,
            actions: actions,
            role: .destructive,
            perform: { showConfirmation = true }
        ) {
            Text("Remove Incomplete Setup")
                .uiTestTruncationProbe()
                .frame(maxWidth: .infinity)
        }
        .buttonStyle(OnboardingSecondaryButtonStyle(
            backgroundColor: Color.red.opacity(0.12),
            foregroundColor: .red.opacity(0.95),
            borderColor: Color.red.opacity(0.22)
        ))
        .confirmationDialog(
            "Remove incomplete Cloud Backup setup?",
            isPresented: $showConfirmation,
            titleVisibility: .visible
        ) {
            Button("Remove Incomplete Setup", role: .destructive) {
                showConfirmation = false
                removeIncompleteSetup()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "This removes only local data from the interrupted setup. Your active Cloud Backup key, cloud data, and wallets on this device will be preserved."
            )
        }
    }
}
