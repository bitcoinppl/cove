import SwiftUI
import UIKit

func cloudBackupPendingEnableCleanupIsAvailable(
    _ state: CloudBackupPendingEnableCleanupState
) -> Bool {
    state == .available
}

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
        CloudBackupExistingBackupAlertHost(
            isPresented: showingExistingBackupPrompt,
            message: existingBackupMessage,
            createNewBackup: acceptCreateNewPasskey,
            useExistingPasskey: acceptExistingPasskey,
            cancel: discardPendingEnable
        ) {
            CloudBackupPasskeyChoiceAlertHost(
                isPresented: showingPasskeyChoice,
                existingPasskeyButtonTitle: existingPasskeyButtonTitle(
                    for: passkeyChoicePresentation?.passkeyHint
                ),
                secondaryActionTitle: passkeyChoicePresentation?.secondaryActionTitle,
                message: passkeyChoicePresentation?.message
                    ?? "Choose how to continue with Cloud Backup.",
                useExistingPasskey: acceptExistingPasskey,
                createNewPasskey: acceptCreateNewPasskey,
                cancel: dismissPasskeyChoice
            ) {
                CloudBackupEnableRootContent(
                    recovery: manager.pendingEnableRecovery,
                    enableFlow: manager.enableFlow,
                    verificationPresentation: manager.verificationPresentation,
                    message: message,
                    isBusy: isBusy,
                    onRemoveIncompleteSetup: confirmPendingEnableCleanup,
                    onConfirmSavedPasskey: confirmSavedPasskey,
                    onCancelConfirmation: cancelSavedPasskeyConfirmation,
                    onEnable: beginEnableChoice,
                    onCancel: onDismiss
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
    }

    private func acceptExistingPasskey() {
        dispatchPromptAction(.acceptEnablePrompt(.useExisting))
    }

    private func acceptCreateNewPasskey() {
        dispatchPromptAction(.acceptEnablePrompt(.createNew))
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
    let recovery: CloudBackupPendingEnableRecovery?
    let enableFlow: CloudBackupEnableFlow?
    let verificationPresentation: CloudBackupVerificationPresentation
    let message: String?
    let isBusy: Bool
    let onRemoveIncompleteSetup: () -> Void
    let onConfirmSavedPasskey: () -> Void
    let onCancelConfirmation: () -> Void
    let onEnable: () -> Void
    let onCancel: () -> Void

    var body: some View {
        ZStack {
            CloudBackupEnableFlowContent(
                recovery: recovery,
                enableFlow: enableFlow,
                message: message,
                isBusy: isBusy,
                onRemoveIncompleteSetup: onRemoveIncompleteSetup,
                onConfirmSavedPasskey: onConfirmSavedPasskey,
                onCancelConfirmation: onCancelConfirmation,
                onEnable: onEnable,
                onCancel: onCancel
            )

            if isBusy {
                CloudBackupEnableBusyOverlay(
                    enableFlow: enableFlow,
                    verificationPresentation: verificationPresentation
                )
            }
        }
    }
}

private struct CloudBackupEnableFlowContent: View {
    private let content: AnyView

    init(
        recovery: CloudBackupPendingEnableRecovery?,
        enableFlow: CloudBackupEnableFlow?,
        message: String?,
        isBusy: Bool,
        onRemoveIncompleteSetup: @escaping () -> Void,
        onConfirmSavedPasskey: @escaping () -> Void,
        onCancelConfirmation: @escaping () -> Void,
        onEnable: @escaping () -> Void,
        onCancel: @escaping () -> Void
    ) {
        if let recovery {
            content = AnyView(CloudBackupPendingEnableRecoveryView(
                recovery: recovery,
                onRemoveIncompleteSetup: onRemoveIncompleteSetup,
                onCancel: onCancel
            ))
        } else if case .awaitingSavedPasskeyConfirmation(.manual) = enableFlow {
            content = AnyView(CloudBackupEnableConfirmationView(
                onContinue: onConfirmSavedPasskey,
                onCancel: onCancelConfirmation
            ))
        } else {
            content = AnyView(CloudBackupEnableOnboardingView(
                onEnable: onEnable,
                onCancel: onCancel,
                message: message,
                isBusy: isBusy
            ))
        }
    }

    var body: some View {
        content
    }
}

private struct CloudBackupPasskeyChoiceAlertHost<Content: View>: View {
    @Binding var isPresented: Bool
    let existingPasskeyButtonTitle: String
    let secondaryActionTitle: String?
    let message: String
    let useExistingPasskey: () -> Void
    let createNewPasskey: () -> Void
    let cancel: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .alert("Passkey Options", isPresented: $isPresented) {
                Button(existingPasskeyButtonTitle, action: useExistingPasskey)

                if let secondaryActionTitle {
                    Button(secondaryActionTitle, action: createNewPasskey)
                }

                Button("Cancel", role: .cancel, action: cancel)
            } message: {
                Text(message)
            }
    }
}

private struct CloudBackupExistingBackupAlertHost<Content: View>: View {
    @Binding var isPresented: Bool
    let message: String
    let createNewBackup: () -> Void
    let useExistingPasskey: () -> Void
    let cancel: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .alert("Existing Cloud Backup Found", isPresented: $isPresented) {
                Button(
                    "Create New Backup",
                    role: .destructive,
                    action: createNewBackup
                )
                Button("Try Existing Passkey", action: useExistingPasskey)
                Button("Cancel", role: .cancel, action: cancel)
            } message: {
                Text(message)
            }
    }
}

struct CloudBackupPendingEnableRecoveryView: View {
    @Environment(\.openURL) private var openURL

    let recovery: CloudBackupPendingEnableRecovery
    let onRemoveIncompleteSetup: () -> Void
    let onCancel: () -> Void

    @State private var showRemovalConfirmation = false

    private var isCleaning: Bool {
        recovery.cleanup == .cleaning
    }

    private var canRemoveIncompleteSetup: Bool {
        cloudBackupPendingEnableCleanupIsAvailable(recovery.cleanup)
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
                        canRemoveIncompleteSetup: canRemoveIncompleteSetup,
                        showRemovalConfirmation: $showRemovalConfirmation
                    )
                }
                .padding()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(CloudBackupEnableBackground())
        .confirmationDialog(
            "Remove incomplete Cloud Backup setup?",
            isPresented: $showRemovalConfirmation,
            titleVisibility: .visible
        ) {
            Button("Remove Incomplete Setup", role: .destructive) {
                onRemoveIncompleteSetup()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "This removes only local data from the interrupted setup. Your active Cloud Backup key, cloud data, and wallets on this device will be preserved."
            )
        }
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
                .foregroundStyle(.white)
            Text("Cloud Backup setup was interrupted and its local recovery records do not match.")
                .font(.footnote)
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
                .foregroundStyle(.coveLightGray)
            Text(supportCode)
                .font(.title3.monospaced().weight(.semibold))
                .foregroundStyle(.white)
                .accessibilityIdentifier("cloudBackup.recovery.supportCode")
            HStack {
                Button(action: copy) {
                    Label("Copy Code", systemImage: "doc.on.doc")
                }
                Spacer()
                Button(action: contactSupport) {
                    Label("Contact Support", systemImage: "envelope")
                }
            }
            .buttonStyle(.bordered)
            .tint(.white)
        }
        .padding(16)
        .background(Color.duskBlue.opacity(0.5))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

private struct CloudBackupRecoveryActions: View {
    let isCleaning: Bool
    let canRemoveIncompleteSetup: Bool
    @Binding var showRemovalConfirmation: Bool

    var body: some View {
        if isCleaning {
            HStack {
                Spacer()
                ProgressView("Removing incomplete setup...")
                    .tint(.white)
                    .foregroundStyle(.white)
                Spacer()
            }
            .padding(.vertical)
        } else if canRemoveIncompleteSetup {
            Button(role: .destructive) {
                showRemovalConfirmation = true
            } label: {
                Text("Remove Incomplete Setup")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingSecondaryButtonStyle(
                backgroundColor: Color.red.opacity(0.12),
                foregroundColor: .red.opacity(0.95),
                borderColor: Color.red.opacity(0.22)
            ))
            .accessibilityIdentifier("cloudBackup.recovery.removeIncompleteSetup")
        }
    }
}
