import MijickPopups
import SwiftUI
import UIKit

@_exported import CoveCore

enum CloudBackupRootPresentation: Equatable {
    case existingBackupFound(CloudBackupEnableContext, CloudBackupPasskeyHint?)
    case passkeyChoice(CloudBackupPasskeyChoiceIntent)
    case missingPasskeyReminder
    case verificationPrompt

    init?(rootPrompt: CloudBackupRootPrompt) {
        switch rootPrompt {
        case .none:
            return nil
        case let .existingBackupFound(context, passkeyHint):
            self = .existingBackupFound(context, passkeyHint)
        case let .passkeyChoice(intent):
            self = .passkeyChoice(intent)
        case .missingPasskeyReminder:
            self = .missingPasskeyReminder
        case .verification:
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
    var isNavigationSettled = true
    var presentationPolicy = CloudBackupPresentationPolicy.requiresUnlockedAuth
}

enum CloudBackupPresentationBlocker: Hashable {
    case settingsLocalModal
    case cloudBackupDetailDialog
}

enum CloudBackupPresentationPolicy: Equatable {
    case requiresUnlockedAuth
    case onboarding

    var requiresUnlockedAuth: Bool {
        self == .requiresUnlockedAuth
    }

    var suppressesGenericPrompts: Bool {
        self == .onboarding
    }
}

func isCloudBackupPresentationPresentable(
    presentation: CloudBackupRootPresentation,
    context: CloudBackupPresentationContext,
    hasBlockers: Bool
) -> Bool {
    guard context.scenePhase == .active else { return false }
    guard !context.presentationPolicy.requiresUnlockedAuth || context.isUnlocked else {
        return false
    }
    guard !context.isCoverPresented else { return false }
    guard !context.appHasAlert else { return false }
    guard !context.appHasSheet else { return false }
    guard !hasBlockers else { return false }
    guard context.isNavigationSettled else { return false }

    switch presentation {
    case .existingBackupFound, .passkeyChoice:
        return true
    case .missingPasskeyReminder, .verificationPrompt:
        guard !context.presentationPolicy.suppressesGenericPrompts else {
            return false
        }

        return !context.isViewingCloudBackup
    }
}

enum CloudBackupVerificationFeedback: Equatable {
    case successFloater(String)
    case failureAlert(title: String, message: String)
}

struct CloudBackupPasskeyChoicePresentation: Equatable {
    let passkeyHint: CloudBackupPasskeyHint?
    let message: String
    let secondaryActionTitle: String?

    init(intent: CloudBackupPasskeyChoiceIntent) {
        switch intent {
        case let .enable(_, passkeyHint):
            self.passkeyHint = passkeyHint
            message = "Would you like to use an existing passkey or start a new backup?"
            secondaryActionTitle = "Start a New Backup"
        case let .enableExistingPasskeyOnly(_, passkeyHint):
            self.passkeyHint = passkeyHint
            message = "Cloud Backup may already exist. Use your existing passkey to continue, or cancel and try again later."
            secondaryActionTitle = nil
        case .repairPasskey:
            passkeyHint = nil
            message = "Would you like to use an existing passkey or create a new one?"
            secondaryActionTitle = "Create New Passkey"
        }
    }
}

private struct CloudBackupSuccessFloater: Identifiable, Equatable {
    let id = UUID()
    let text: String
}

func cloudBackupVerificationFeedback(
    for presentation: CloudBackupVerificationPresentation
) -> CloudBackupVerificationFeedback? {
    switch presentation {
    case .completed(source: .rootPrompt):
        .successFloater("Cloud Backup Verified")
    case let .failed(source: .rootPrompt, message: message):
        .failureAlert(
            title: "Cloud Backup Verification Failed",
            message: message
        )
    default:
        nil
    }
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
    @ObservationIgnored private let rootPrompt: () -> CloudBackupRootPrompt

    private(set) var currentPresentation: CloudBackupRootPresentation?
    private(set) var queuedPresentation: CloudBackupRootPresentation?

    init(rootPrompt: @escaping () -> CloudBackupRootPrompt = { CloudBackupManager.shared.rootPrompt }) {
        self.rootPrompt = rootPrompt
    }

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
            rootPrompt: rootPrompt()
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
            if blockers.contains(.settingsLocalModal) {
                requiresPresentationDelay = true
            }
            if
                currentPresentation == desiredPresentation,
                isPromptBlockedOnlyByNavigationSettling(desiredPresentation)
            {
                return
            }
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
        isCloudBackupPresentationPresentable(
            presentation: presentation,
            context: context,
            hasBlockers: !blockers.isEmpty
        )
    }

    private func isPromptBlockedOnlyByNavigationSettling(
        _ presentation: CloudBackupRootPresentation
    ) -> Bool {
        guard !context.isNavigationSettled else { return false }

        var settledContext = context
        settledContext.isNavigationSettled = true
        return isCloudBackupPresentationPresentable(
            presentation: presentation,
            context: settledContext,
            hasBlockers: !blockers.isEmpty
        )
    }

    private func resumeQueuedPresentation() {
        transitionTask = nil

        guard let queuedPresentation else { return }
        guard
            CloudBackupRootPresentation(rootPrompt: rootPrompt())
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase

    let app: AppManager
    let auth: AuthManager
    let isCoverPresented: Bool
    let presentationPolicy: CloudBackupPresentationPolicy
    let content: Content

    @State private var manager = CloudBackupManager.shared
    @State private var coordinator = CloudBackupPresentationCoordinator()
    @State private var successFloater: CloudBackupSuccessFloater?
    @State private var successFloaterDismissTask: Task<Void, Never>?

    init(
        app: AppManager,
        auth: AuthManager,
        isCoverPresented: Bool,
        presentationPolicy: CloudBackupPresentationPolicy = .requiresUnlockedAuth,
        @ViewBuilder content: () -> Content
    ) {
        self.app = app
        self.auth = auth
        self.isCoverPresented = isCoverPresented
        self.presentationPolicy = presentationPolicy
        self.content = content()
    }

    private var showingExistingBackupPrompt: Binding<Bool> {
        Binding(
            get: {
                if case .existingBackupFound = coordinator.currentPresentation { return true }
                return false
            },
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

    private var existingBackupPasskeyHint: CloudBackupPasskeyHint? {
        if case let .existingBackupFound(_, passkeyHint) = coordinator.currentPresentation {
            return passkeyHint
        }

        return nil
    }

    private var passkeyChoiceIntent: CloudBackupPasskeyChoiceIntent? {
        if case let .passkeyChoice(intent) = coordinator.currentPresentation {
            return intent
        }

        return nil
    }

    private var passkeyChoicePresentation: CloudBackupPasskeyChoicePresentation? {
        guard let passkeyChoiceIntent else { return nil }
        return CloudBackupPasskeyChoicePresentation(intent: passkeyChoiceIntent)
    }

    private var presentationContext: CloudBackupPresentationContext {
        CloudBackupPresentationContext(
            scenePhase: scenePhase,
            isUnlocked: auth.lockState == .unlocked,
            isCoverPresented: isCoverPresented,
            appHasAlert: app.alertState != nil,
            appHasSheet: app.sheetState != nil,
            isViewingCloudBackup: app.currentRoute.isEqual(routeToCheck: .settings(.cloudBackup)),
            isNavigationSettled: app.isNavigationSettled,
            presentationPolicy: presentationPolicy
        )
    }

    private func handlePasskeyChoice(existing: Bool) {
        guard let intent = passkeyChoiceIntent else { return }
        coordinator.dismissCurrentPresentation()

        switch (intent, existing) {
        case (.enable, true):
            manager.dispatch(action: .acceptEnablePrompt(.useExisting))
        case (.enable, false):
            manager.dispatch(action: .acceptEnablePrompt(.createNew))
        case (.enableExistingPasskeyOnly, true):
            manager.dispatch(action: .acceptEnablePrompt(.useExisting))
        case (.enableExistingPasskeyOnly, false):
            return
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

    private func existingPasskeyButtonTitle(for hint: CloudBackupPasskeyHint?) -> String {
        guard let hint else { return "Use Existing Passkey" }
        return "Use Existing Passkey (\(hint.nameSuffix))"
    }

    private func existingBackupMessage(for hint: CloudBackupPasskeyHint?) -> String {
        guard let hint else {
            return "Creating a new Cloud Backup will not include wallets from your previous backup. If you still have access to the passkey for that backup, use the existing passkey instead."
        }

        return "Creating a new Cloud Backup will not include wallets from your previous backup. If you still have access to the passkey named Cove Cloud Backup (\(hint.nameSuffix)), use that passkey instead."
    }

    private func showSuccessFloater(_ text: String) {
        successFloaterDismissTask?.cancel()

        let floater = CloudBackupSuccessFloater(text: text)
        successFloater = floater
        UIAccessibility.post(notification: .announcement, argument: text)
        successFloaterDismissTask = Task {
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }

            await MainActor.run {
                dismissSuccessFloater(id: floater.id)
            }
        }
    }

    private func dismissSuccessFloater(id: UUID? = nil) {
        if let id, successFloater?.id != id {
            return
        }

        successFloaterDismissTask?.cancel()
        successFloaterDismissTask = nil
        successFloater = nil
    }

    @ViewBuilder
    private var successFloaterOverlay: some View {
        if let successFloater {
            FloaterPopupView(text: successFloater.text)
                .padding(.top, 14)
                .gesture(
                    DragGesture()
                        .onEnded { gesture in
                            if abs(gesture.translation.width) > 40 || abs(gesture.translation.height) > 40 {
                                dismissSuccessFloater(id: successFloater.id)
                            }
                        }
                )
                .transition(reduceMotion ? .identity : .move(edge: .top).combined(with: .opacity))
                .zIndex(1)
        }
    }

    private func handleVerificationPresentation(_ presentation: CloudBackupVerificationPresentation) {
        guard let feedback = cloudBackupVerificationFeedback(for: presentation) else { return }

        switch feedback {
        case let .successFloater(text):
            showSuccessFloater(text)
        case let .failureAlert(title, message):
            app.alertState = .init(.general(title: title, message: message))
        }
    }

    var body: some View {
        content
            .overlay(alignment: .top) {
                successFloaterOverlay
            }
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: successFloater)
            .environment(coordinator)
            .onChange(of: presentationContext, initial: true) { _, context in
                coordinator.update(context: context)
            }
            .onChange(of: manager.rootPrompt) { _, _ in
                coordinator.reconcile()
            }
            .onChange(of: manager.verificationState) { _, _ in
                coordinator.reconcile()
            }
            .onChange(of: manager.verificationPresentation) { _, presentation in
                handleVerificationPresentation(presentation)
            }
            .onDisappear {
                dismissSuccessFloater()
            }
            .alert(
                "Existing Cloud Backup Found",
                isPresented: showingExistingBackupPrompt
            ) {
                Button("Create New Backup", role: .destructive) {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(action: .acceptEnablePrompt(.createNew))
                }
                Button("Try Existing Passkey") {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(action: .acceptEnablePrompt(.useExisting))
                }
                Button("Cancel", role: .cancel) {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(action: .discardPendingEnableCloudBackup)
                }
            } message: {
                Text(existingBackupMessage(for: existingBackupPasskeyHint))
            }
            .alert(
                "Passkey Options",
                isPresented: showingPasskeyChoicePrompt
            ) {
                Button(existingPasskeyButtonTitle(for: passkeyChoicePresentation?.passkeyHint)) {
                    handlePasskeyChoice(existing: true)
                }
                if let secondaryActionTitle = passkeyChoicePresentation?.secondaryActionTitle {
                    Button(secondaryActionTitle) {
                        handlePasskeyChoice(existing: false)
                    }
                }
                Button("Cancel", role: .cancel) {
                    coordinator.dismissCurrentPresentation()
                    manager.dispatch(action: .dismissPasskeyChoicePrompt)
                }
            } message: {
                Text(passkeyChoicePresentation?.message ?? "Choose how to continue with Cloud Backup.")
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
                        coordinator.dismissCurrentPresentation()
                        manager.startVerification(source: .rootPrompt)
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
        if case .running = manager.verificationState { return true }
        return false
    }

    private var failure: DeepVerificationFailure? {
        guard !manager.shouldPromptVerification else { return nil }
        if case let .failed(failure) = manager.verificationState { return failure }
        return nil
    }

    private var title: String {
        if isVerifying { return "Verifying Cloud Backup" }
        if failure != nil { return "Verification Failed" }

        return "Verify"
    }

    private var message: String {
        if let failure { return failure.message() }
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

                    Button(action: onDismiss) {
                        Image(systemName: "xmark")
                            .font(.headline)
                            .foregroundStyle(.white.opacity(0.85))
                            .frame(width: 44, height: 44)
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

                    Button(isVerifying ? "Hide" : "Not Now", action: onDismiss)
                        .buttonStyle(OnboardingSecondaryButtonStyle())
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
