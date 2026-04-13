import SwiftUI

@_exported import CoveCore

private enum CloudBackupRootPresentation: Equatable {
    case existingBackupFound
    case passkeyChoice(CloudBackupPasskeyChoiceFlow)
    case missingPasskeyReminder
    case verificationPrompt

    init?(promptIntent: CloudBackupPromptIntent) {
        switch promptIntent {
        case .none:
            return nil
        case .existingBackupFound:
            self = .existingBackupFound
        case let .passkeyChoice(flow):
            self = .passkeyChoice(flow)
        case .missingPasskeyReminder:
            self = .missingPasskeyReminder
        case .verificationPrompt:
            self = .verificationPrompt
        }
    }
}

struct CloudBackupPresentationContext: Equatable {
    var scenePhase: ScenePhase = .background
    var isUnlocked = false
    var isCoverPresented = true
    var appHasAlert = false
    var appHasSheet = false
    var isViewingCloudBackup = false
}

enum CloudBackupPresentationBlocker: Hashable {
    case settingsLocalModal
    case cloudBackupDetailDialog
}

@MainActor
@Observable
final class CloudBackupPresentationCoordinator {
    private static let presentationDelayNs: UInt64 = 800_000_000

    @ObservationIgnored private var transitionTask: Task<Void, Never>?
    @ObservationIgnored private var ignoreNextDismissEvent = false
    @ObservationIgnored private var requiresPresentationDelay = false
    @ObservationIgnored private var context = CloudBackupPresentationContext()
    @ObservationIgnored private var blockers: Set<CloudBackupPresentationBlocker> = []

    fileprivate var currentPresentation: CloudBackupRootPresentation?
    private var queuedPresentation: CloudBackupRootPresentation?

    func update(context: CloudBackupPresentationContext) {
        self.context = context
        reconcile()
    }

    func setBlocker(_ blocker: CloudBackupPresentationBlocker, active: Bool) {
        if active {
            blockers.insert(blocker)
        } else {
            blockers.remove(blocker)
        }

        reconcile()
    }

    func dismissCurrentPresentation() {
        transitionTask?.cancel()
        transitionTask = nil
        queuedPresentation = nil
        guard currentPresentation != nil else { return }
        requiresPresentationDelay = true
        ignoreNextDismissEvent = true
        currentPresentation = nil
    }

    func consumeDismissEvent() -> Bool {
        if ignoreNextDismissEvent {
            ignoreNextDismissEvent = false
            return true
        }

        return false
    }

    func reconcile() {
        let desiredPresentation = CloudBackupRootPresentation(
            promptIntent: CloudBackupManager.shared.promptIntent
        )

        guard let desiredPresentation else {
            requiresPresentationDelay = false
            clearVisiblePresentation()
            return
        }

        if !isPromptPresentable(desiredPresentation) {
            transitionTask?.cancel()
            transitionTask = nil
            queuedPresentation = desiredPresentation
            if currentPresentation != nil {
                ignoreNextDismissEvent = true
                currentPresentation = nil
            }
            return
        }

        if currentPresentation == desiredPresentation {
            transitionTask?.cancel()
            transitionTask = nil
            queuedPresentation = nil
            requiresPresentationDelay = false
            return
        }

        if currentPresentation == nil {
            transitionTask?.cancel()
            transitionTask = nil
            if requiresPresentationDelay {
                queuedPresentation = desiredPresentation
                scheduleQueuedPresentation()
            } else {
                queuedPresentation = nil
                currentPresentation = desiredPresentation
            }
            return
        }

        queuedPresentation = desiredPresentation
        requiresPresentationDelay = true
        ignoreNextDismissEvent = true
        currentPresentation = nil
        scheduleQueuedPresentation()
    }

    private func clearVisiblePresentation() {
        transitionTask?.cancel()
        transitionTask = nil
        queuedPresentation = nil
        guard currentPresentation != nil else { return }
        requiresPresentationDelay = true
        ignoreNextDismissEvent = true
        currentPresentation = nil
    }

    private func scheduleQueuedPresentation() {
        transitionTask?.cancel()
        transitionTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.presentationDelayNs)
            self?.resumeQueuedPresentation()
        }
    }

    private func isPromptPresentable(_ presentation: CloudBackupRootPresentation) -> Bool {
        guard context.scenePhase == .active else { return false }
        guard context.isUnlocked else { return false }
        guard !context.isCoverPresented else { return false }
        guard !context.appHasAlert else { return false }
        guard !context.appHasSheet else { return false }
        guard blockers.isEmpty else { return false }

        switch presentation {
        case .existingBackupFound, .passkeyChoice:
            return true
        case .missingPasskeyReminder, .verificationPrompt:
            return !context.isViewingCloudBackup
        }
    }

    private func resumeQueuedPresentation() {
        transitionTask = nil

        guard let queuedPresentation else { return }
        guard
            CloudBackupRootPresentation(promptIntent: CloudBackupManager.shared.promptIntent)
            == queuedPresentation
        else {
            self.queuedPresentation = nil
            return
        }
        guard isPromptPresentable(queuedPresentation) else { return }

        requiresPresentationDelay = false
        currentPresentation = queuedPresentation
        self.queuedPresentation = nil
    }
}

struct CloudBackupPresentationHost<Content: View>: View {
    @Environment(\.scenePhase) private var scenePhase

    let app: AppManager
    let auth: AuthManager
    let isCoverPresented: Bool
    let content: Content

    @State private var manager = CloudBackupManager.shared
    @State private var coordinator = CloudBackupPresentationCoordinator()

    init(
        app: AppManager,
        auth: AuthManager,
        isCoverPresented: Bool,
        @ViewBuilder content: () -> Content
    ) {
        self.app = app
        self.auth = auth
        self.isCoverPresented = isCoverPresented
        self.content = content()
    }

    private var showingExistingBackupPrompt: Binding<Bool> {
        Binding(
            get: { coordinator.currentPresentation == .existingBackupFound },
            set: { isPresented in
                guard !isPresented else { return }
                if coordinator.consumeDismissEvent() { return }
                manager.dispatch(action: .discardPendingEnableCloudBackup)
            }
        )
    }

    private var showingPasskeyChoicePrompt: Binding<Bool> {
        Binding(
            get: {
                if case .passkeyChoice = coordinator.currentPresentation { return true }
                return false
            },
            set: { isPresented in
                guard !isPresented else { return }
                if coordinator.consumeDismissEvent() { return }
                manager.dispatch(action: .dismissPasskeyChoicePrompt)
            }
        )
    }

    private var showingMissingPasskeyReminder: Binding<Bool> {
        Binding(
            get: { coordinator.currentPresentation == .missingPasskeyReminder },
            set: { isPresented in
                guard !isPresented else { return }
                if coordinator.consumeDismissEvent() { return }
                manager.dispatch(action: .dismissMissingPasskeyReminder)
            }
        )
    }

    private var showingVerificationPrompt: Binding<Bool> {
        Binding(
            get: { coordinator.currentPresentation == .verificationPrompt },
            set: { isPresented in
                guard !isPresented else { return }
                if coordinator.consumeDismissEvent() { return }
                manager.dispatch(action: .dismissVerificationPrompt)
            }
        )
    }

    private var passkeyChoiceFlow: CloudBackupPasskeyChoiceFlow? {
        if case let .passkeyChoice(flow) = coordinator.currentPresentation {
            return flow
        }

        return nil
    }

    private var presentationContext: CloudBackupPresentationContext {
        CloudBackupPresentationContext(
            scenePhase: scenePhase,
            isUnlocked: auth.lockState == .unlocked,
            isCoverPresented: isCoverPresented,
            appHasAlert: app.alertState != nil,
            appHasSheet: app.sheetState != nil,
            isViewingCloudBackup: app.currentRoute.isEqual(routeToCheck: .settings(.cloudBackup))
        )
    }

    private func handlePasskeyChoice(existing: Bool) {
        guard let flow = passkeyChoiceFlow else { return }
        coordinator.dismissCurrentPresentation()

        switch (flow, existing) {
        case (.enable, true):
            manager.dispatch(action: .enableCloudBackup)
        case (.enable, false):
            manager.dispatch(action: .enableCloudBackupNoDiscovery)
        case (.repairPasskey, true):
            manager.dispatch(action: .repairPasskey)
        case (.repairPasskey, false):
            manager.dispatch(action: .repairPasskeyNoDiscovery)
        }
    }

    private func openCloudBackupScreen() {
        coordinator.dismissCurrentPresentation()

        let route = Route.settings(.cloudBackup)
        if app.currentRoute.isEqual(routeToCheck: route) {
            return
        }

        app.pushRoute(route)
    }

    var body: some View {
        content
            .environment(coordinator)
            .onChange(of: presentationContext, initial: true) { _, context in
                coordinator.update(context: context)
            }
            .onChange(of: manager.promptIntent) { _, _ in
                coordinator.reconcile()
            }
            .confirmationDialog(
                "Existing Cloud Backup Found",
                isPresented: showingExistingBackupPrompt
            ) {
                Button("Create New Backup", role: .destructive) {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(action: .enableCloudBackupForceNew)
                }
                Button("Cancel", role: .cancel) {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(action: .discardPendingEnableCloudBackup)
                }
            } message: {
                Text("Creating a new backup will not include wallets from the previous one.")
            }
            .alert(
                "Passkey Options",
                isPresented: showingPasskeyChoicePrompt
            ) {
                Button("Use Existing Passkey") {
                    handlePasskeyChoice(existing: true)
                }
                Button("Create New Passkey") {
                    handlePasskeyChoice(existing: false)
                }
                Button("Cancel", role: .cancel) {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(action: .dismissPasskeyChoicePrompt)
                }
            } message: {
                Text("Would you like to use an existing passkey or create a new one?")
            }
            .alert(
                "Cloud Backup Passkey Missing",
                isPresented: showingMissingPasskeyReminder
            ) {
                Button("Open Cloud Backup") {
                    openCloudBackupScreen()
                }
                Button("Not Now", role: .cancel) {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(action: .dismissMissingPasskeyReminder)
                }
            } message: {
                Text(
                    "Add a new passkey to restore access to your cloud backup. Until you do, your backups can't be restored."
                )
            }
            .fullScreenCover(isPresented: showingVerificationPrompt) {
                CloudBackupVerificationPromptView(
                    onDismiss: {
                        coordinator.dismissCurrentPresentation()
                        manager.dispatch(action: .dismissVerificationPrompt)
                    },
                    onVerify: {
                        manager.dispatch(action: .startVerification)
                    }
                )
                .interactiveDismissDisabled(true)
            }
    }
}

private struct CloudBackupVerificationPromptView: View {
    @State private var manager = CloudBackupManager.shared

    let onDismiss: () -> Void
    let onVerify: () -> Void

    private var isVerifying: Bool {
        if case .verifying = manager.verification { return true }
        return false
    }

    private var failure: DeepVerificationFailure? {
        guard !manager.shouldPromptVerification else { return nil }
        if case let .failed(failure) = manager.verification { return failure }
        return nil
    }

    private var title: String {
        if isVerifying { return "Verifying Cloud Backup" }
        if failure != nil { return "Verification Failed" }

        return "Verify"
    }

    private var message: String {
        if let failure { return failure.message }
        if isVerifying { return "Confirming your updated cloud backup can be decrypted and restored. Continuing may ask for your passkey." }

        return "Verify your updated cloud backup now to confirm it is accessible. Continuing may ask for your passkey."
    }

    private var primaryButtonTitle: String {
        failure == nil ? "Verify" : "Try Again"
    }

    private var heroIconName: String {
        failure == nil ? "checkmark.shield.fill" : "exclamationmark.triangle.fill"
    }

    private var heroTint: Color {
        failure == nil ? .btnGradientLight : .orange
    }

    private var heroFillColor: Color {
        failure == nil ? Color.duskBlue.opacity(0.42) : Color.orange.opacity(0.12)
    }

    var body: some View {
        ScrollView(.vertical) {
            VStack(spacing: 0) {
                HStack {
                    Spacer()

                    if !isVerifying {
                        Button(action: onDismiss) {
                            Image(systemName: "xmark")
                                .font(.headline)
                                .foregroundStyle(.white.opacity(0.85))
                                .frame(width: 44, height: 44)
                        }
                    }
                }
                .padding(.top, 4)

                Spacer()
                    .frame(height: 20)

                heroView

                Spacer()
                    .frame(height: 36)

                VStack(spacing: 12) {
                    HStack {
                        Text(title)
                            .font(.system(size: 38, weight: .semibold))
                            .foregroundStyle(.white)

                        Spacer()
                    }

                    HStack {
                        Text(message)
                            .font(OnboardingRecoveryTypography.body)
                            .foregroundStyle(.coveLightGray.opacity(0.76))
                            .fixedSize(horizontal: false, vertical: true)

                        Spacer()
                    }
                }

                Spacer()
                    .frame(height: 28)

                VStack(spacing: 12) {
                    Button(action: onVerify) {
                        HStack {
                            if isVerifying {
                                ProgressView()
                                    .tint(.midnightBlue)
                                    .padding(.trailing, 6)
                            }

                            Text(primaryButtonTitle)
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(OnboardingPrimaryButtonStyle())
                    .disabled(isVerifying)

                    if !isVerifying {
                        Button("Not Now", action: onDismiss)
                            .buttonStyle(OnboardingSecondaryButtonStyle())
                    }
                }

                Spacer()
                    .frame(height: 20)
            }
            .padding(.horizontal)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(backgroundView)
    }

    private var heroView: some View {
        ZStack {
            Circle()
                .fill(heroFillColor)
                .frame(width: 108, height: 108)

            Circle()
                .stroke(heroTint.opacity(0.36), lineWidth: 1)
                .frame(width: 108, height: 108)

            Image(systemName: heroIconName)
                .font(.system(size: 34, weight: .medium))
                .foregroundStyle(heroTint)
        }
    }

    private var backgroundView: some View {
        ZStack {
            Color.midnightBlue

            RadialGradient(
                stops: [
                    .init(color: Color.duskBlue.opacity(0.90), location: 0),
                    .init(color: Color.duskBlue.opacity(0.28), location: 0.42),
                    .init(color: .clear, location: 0.82),
                ],
                center: .init(x: 0.30, y: 0.16),
                startRadius: 0,
                endRadius: 380
            )

            RadialGradient(
                stops: [
                    .init(color: heroTint.opacity(0.16), location: 0),
                    .init(color: .clear, location: 0.70),
                ],
                center: .init(x: 0.76, y: 0.12),
                startRadius: 0,
                endRadius: 240
            )
        }
        .ignoresSafeArea()
    }
}
