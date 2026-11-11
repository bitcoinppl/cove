@_exported import CoveCore
import MijickPopups
import SwiftUI
import UIKit

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

/// The passkey-choice prompt's copy and its choices, which Rust's entries then bind
struct CloudBackupPasskeyChoicePresentation {
    let passkeyHint: CloudBackupPasskeyHint?
    let message: String
    let choices: [CloudBackupPromptChoice]

    init(intent: CloudBackupPasskeyChoiceIntent) {
        switch intent {
        case let .enable(_, passkeyHint):
            self.passkeyHint = passkeyHint
            message = "Would you like to use an existing passkey or start a new backup?"
        case let .enableExistingPasskeyOnly(_, passkeyHint):
            self.passkeyHint = passkeyHint
            message = "Cloud Backup may already exist. Use your existing passkey to continue, or cancel and try again later."
        case .repairPasskey:
            passkeyHint = nil
            message = "Would you like to use an existing passkey or create a new one?"
        }

        choices = CloudBackupPromptChoice.passkeyChoice(intent, hint: passkeyHint)
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
    @ObservationIgnored private var ignoreNextDismissEvent = false
    @ObservationIgnored private var context = CloudBackupPresentationContext()
    @ObservationIgnored private var blockers: Set<CloudBackupPresentationBlocker> = []
    @ObservationIgnored private let rootPrompt: () -> CloudBackupRootPrompt

    let presentationTransitions =
        PresentationTransitionCoordinator<CloudBackupRootPresentation>()

    var currentPresentation: CloudBackupRootPresentation? {
        presentationTransitions.currentPresentation?.item
    }

    var queuedPresentation: CloudBackupRootPresentation? {
        presentationTransitions.queuedPresentation
    }

    init(rootPrompt: @escaping () -> CloudBackupRootPrompt = { CloudBackupManager.shared.screen.rootPrompt }) {
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
        guard currentPresentation != nil else { return }

        ignoreNextDismissEvent = true
        presentationTransitions.discard { _ in true }
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
            clearVisiblePresentation()
            return
        }

        if !isPromptPresentable(desiredPresentation) {
            if
                currentPresentation == desiredPresentation,
                isPromptBlockedOnlyByNavigationSettling(desiredPresentation)
            {
                presentationTransitions.discardQueued { _ in true }
                return
            }

            presentationTransitions.queue(desiredPresentation)
            if currentPresentation != nil {
                ignoreNextDismissEvent = true
                presentationTransitions.dismissCurrentPresentation()
            }
            return
        }

        if currentPresentation == desiredPresentation {
            presentationTransitions.discardQueued { _ in true }
            return
        }

        if currentPresentation == nil {
            if presentationTransitions.isAwaitingPresenterReadiness {
                presentationTransitions.queue(desiredPresentation)
            } else {
                presentationTransitions.present(desiredPresentation)
            }
            return
        }

        ignoreNextDismissEvent = true
        presentationTransitions.transition(to: desiredPresentation)
    }

    func presenterDidBecomeReady(_ requestID: UUID) {
        let desiredPresentation = CloudBackupRootPresentation(rootPrompt: rootPrompt())
        let canPresentQueuedPresentation = queuedPresentation == desiredPresentation
            && queuedPresentation.map(isPromptPresentable) == true

        presentationTransitions.presenterDidBecomeReady(
            requestID,
            presentQueuedPresentation: canPresentQueuedPresentation
        )
        reconcile()
    }

    func hostDidDisappear() {
        if currentPresentation != nil {
            ignoreNextDismissEvent = true
        }

        presentationTransitions.hostDidDisappear()
    }

    private func clearVisiblePresentation() {
        if currentPresentation != nil {
            ignoreNextDismissEvent = true
        }

        presentationTransitions.discard { _ in true }
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

    /// An alert closes on any tap, so the prompt goes down here; when Rust rejects the choice it
    /// still projects the prompt, and the rejection brings the prompt back with it
    private func choose(_ choice: CloudBackupPromptChoice) {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: choice.intent)
    }

    private func openCloudBackupScreen() {
        coordinator.dismissCurrentPresentation()

        let route = Route.settings(.cloudBackup)
        if app.currentRoute.isEqual(routeToCheck: route) {
            return
        }

        app.pushRoute(route)
    }

    private func cancelExistingBackupPrompt() {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .discardPendingEnableCloudBackup)
    }

    private func cancelPasskeyChoice() {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .dismissPasskeyChoicePrompt)
    }

    private func dismissMissingPasskeyReminder() {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .dismissMissingPasskeyReminder)
    }

    private func dismissVerificationPrompt() {
        coordinator.dismissCurrentPresentation()
        manager.dispatch(action: .dismissVerificationPrompt)
    }

    /// The cover stays until Rust stops projecting the prompt, so a rejected start is drawn in it
    private func verifyCloudBackup() {
        manager.dispatch(action: .startVerification(.rootPrompt))
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
                CloudBackupSuccessFloaterOverlay(
                    floater: successFloater,
                    reduceMotion: reduceMotion,
                    dismiss: dismissSuccessFloater
                )
            }
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: successFloater)
            .environment(coordinator)
            .presentationTransitionHost(
                state: coordinator.presentationTransitions.hostState,
                presenterDidBecomeReady: coordinator.presenterDidBecomeReady,
                hostDidDisappear: coordinator.hostDidDisappear
            )
            .modifier(CloudBackupObservationModifier(
                presentationContext: presentationContext,
                rootPrompt: manager.screen.rootPrompt,
                lastOutcome: manager.screen.lastOutcome,
                verificationState: manager.screen.verification?.state,
                verificationPresentation: manager.screen.verification?.presentation
                    ?? .hidden(source: nil),
                updateContext: coordinator.update,
                reconcile: coordinator.reconcile,
                handleVerificationPresentation: handleVerificationPresentation,
                onDisappear: dismissSuccessFloater
            ))
            .modifier(CloudBackupPromptAlertModifier(
                title: "Existing Cloud Backup Found",
                isPresented: showingExistingBackupPrompt,
                message: existingBackupMessage(for: existingBackupPasskeyHint),
                choices: CloudBackupPromptChoice.existingBackup,
                screen: manager.screen,
                choose: choose,
                cancel: cancelExistingBackupPrompt
            ))
            .modifier(CloudBackupPromptAlertModifier(
                title: "Passkey Options",
                isPresented: showingPasskeyChoicePrompt,
                message: passkeyChoicePresentation?.message ?? "Choose how to continue with Cloud Backup.",
                choices: passkeyChoicePresentation?.choices ?? [],
                screen: manager.screen,
                choose: choose,
                cancel: cancelPasskeyChoice
            ))
            .modifier(MissingPasskeyAlertModifier(
                isPresented: showingMissingPasskeyReminder,
                openCloudBackup: openCloudBackupScreen,
                dismiss: dismissMissingPasskeyReminder
            ))
            .modifier(VerificationPromptModifier(
                isPresented: showingVerificationPrompt,
                dismiss: dismissVerificationPrompt,
                verify: verifyCloudBackup
            ))
    }
}

private struct CloudBackupSuccessFloaterOverlay: View {
    let floater: CloudBackupSuccessFloater?
    let reduceMotion: Bool
    let dismiss: (UUID?) -> Void

    var body: some View {
        if let floater {
            FloaterPopupView(text: floater.text)
                .padding(.top, 14)
                .gesture(
                    DragGesture()
                        .onEnded { gesture in
                            let horizontalDistance = abs(gesture.translation.width)
                            let verticalDistance = abs(gesture.translation.height)
                            guard horizontalDistance > 40 || verticalDistance > 40 else { return }

                            dismiss(floater.id)
                        }
                )
                .transition(reduceMotion ? .identity : .move(edge: .top).combined(with: .opacity))
                .zIndex(1)
        }
    }
}

private struct CloudBackupObservationModifier: ViewModifier {
    let presentationContext: CloudBackupPresentationContext
    let rootPrompt: CloudBackupRootPrompt
    let lastOutcome: CloudBackupLastOutcome?
    let verificationState: CloudBackupVerificationState?
    let verificationPresentation: CloudBackupVerificationPresentation
    let updateContext: (CloudBackupPresentationContext) -> Void
    let reconcile: () -> Void
    let handleVerificationPresentation: (CloudBackupVerificationPresentation) -> Void
    let onDisappear: (UUID?) -> Void

    func body(content: Content) -> some View {
        content
            .onChange(of: presentationContext, initial: true) { _, context in
                updateContext(context)
            }
            .onChange(of: rootPrompt) { _, _ in
                reconcile()
            }
            // a rejected choice leaves the prompt Rust projects, which the tap already took down
            .onChange(of: lastOutcome) { _, outcome in
                if outcome != nil { reconcile() }
            }
            .onChange(of: verificationState) { _, _ in
                reconcile()
            }
            .onChange(of: verificationPresentation) { _, presentation in
                handleVerificationPresentation(presentation)
            }
            .onDisappear {
                onDisappear(nil)
            }
    }
}

/// A root prompt alert whose buttons are the choices Rust offers for it now
struct CloudBackupPromptAlertModifier: ViewModifier {
    let title: String
    @Binding var isPresented: Bool
    let message: String
    let choices: [CloudBackupPromptChoice]
    let screen: CloudBackupScreen
    let choose: (CloudBackupPromptChoice) -> Void
    let cancel: () -> Void

    func body(content: Content) -> some View {
        let offered = choices.offered(on: screen)

        content.alert(title, isPresented: $isPresented) {
            CloudBackupPromptChoiceButtons(choices: offered, choose: choose, cancel: cancel)
        } message: {
            Text(offered.message(message, on: screen))
        }
    }
}

private struct MissingPasskeyAlertModifier: ViewModifier {
    @Binding var isPresented: Bool
    let openCloudBackup: () -> Void
    let dismiss: () -> Void

    func body(content: Content) -> some View {
        content.alert("Cloud Backup Passkey Missing", isPresented: $isPresented) {
            Button("Open Cloud Backup", action: openCloudBackup)
            Button("Not Now", role: .cancel, action: dismiss)
        } message: {
            Text(
                "Add a new passkey to restore access to your cloud backup. Until you do, your backups can't be restored."
            )
        }
    }
}

private struct VerificationPromptModifier: ViewModifier {
    @Binding var isPresented: Bool
    let dismiss: () -> Void
    let verify: () -> Void

    func body(content: Content) -> some View {
        content.fullScreenCover(isPresented: $isPresented) {
            CloudBackupVerificationPromptView(
                onDismiss: dismiss,
                onVerify: verify
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
        if case .running = manager.screen.verification?.state { return true }
        return false
    }

    private var failure: DeepVerificationFailure? {
        if case let .failed(failure) = manager.screen.verification?.state { return failure }
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

    /// The outcome of a Verify that didn't simply start, drawn under the button that sent it
    private var verifyOutcome: CloudBackupOutcome? {
        guard let lastOutcome = manager.screen.lastOutcome, case .action(.verify) = lastOutcome.target else {
            return nil
        }

        return lastOutcome.outcome
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
        CloudBackupVerificationPromptContent(
            title: title,
            message: message,
            primaryButtonTitle: primaryButtonTitle,
            heroIconName: heroIconName,
            heroTint: heroTint,
            heroFillColor: heroFillColor,
            isVerifying: isVerifying,
            verifyAvailability: manager.screen.actions.availability(action: .verify),
            verifyOutcome: verifyOutcome,
            onDismiss: onDismiss,
            onVerify: onVerify
        )
        .background(CloudBackupVerificationBackground(heroTint: heroTint))
        .accessibilityIdentifier(CloudBackupUiElement.promptVerification.identifier)
    }
}

private struct CloudBackupVerificationPromptContent: View {
    let title: String
    let message: String
    let primaryButtonTitle: String
    let heroIconName: String
    let heroTint: Color
    let heroFillColor: Color
    let isVerifying: Bool
    let verifyAvailability: CloudBackupActionAvailability?
    let verifyOutcome: CloudBackupOutcome?
    let onDismiss: () -> Void
    let onVerify: () -> Void

    var body: some View {
        ScrollView(.vertical) {
            VStack(spacing: 0) {
                CloudBackupVerificationCloseButton(action: onDismiss)

                Spacer()
                    .frame(height: 20)

                CloudBackupVerificationHero(
                    iconName: heroIconName,
                    tint: heroTint,
                    fillColor: heroFillColor
                )

                Spacer()
                    .frame(height: 36)

                CloudBackupVerificationCopy(title: title, message: message)

                Spacer()
                    .frame(height: 28)

                CloudBackupVerificationActions(
                    primaryButtonTitle: primaryButtonTitle,
                    isVerifying: isVerifying,
                    verifyAvailability: verifyAvailability,
                    verifyOutcome: verifyOutcome,
                    verify: onVerify,
                    dismiss: onDismiss
                )

                Spacer()
                    .frame(height: 20)
            }
            .padding(.horizontal)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct CloudBackupVerificationCloseButton: View {
    let action: () -> Void

    var body: some View {
        HStack {
            Spacer()

            Button(action: action) {
                Image(systemName: "xmark")
                    .font(.headline)
                    .foregroundStyle(.white.opacity(0.85))
                    .frame(width: 44, height: 44)
            }
        }
        .padding(.top, 4)
    }
}

private struct CloudBackupVerificationHero: View {
    let iconName: String
    let tint: Color
    let fillColor: Color

    var body: some View {
        ZStack {
            Circle()
                .fill(fillColor)
                .frame(width: 108, height: 108)

            Circle()
                .stroke(tint.opacity(0.36), lineWidth: 1)
                .frame(width: 108, height: 108)

            Image(systemName: iconName)
                .font(.system(size: 34, weight: .medium))
                .foregroundStyle(tint)
        }
    }
}

private struct CloudBackupVerificationCopy: View {
    let title: String
    let message: String

    var body: some View {
        VStack(spacing: 12) {
            Text(title)
                .font(.system(size: 38, weight: .semibold))
                .foregroundStyle(.white)
                .frame(maxWidth: .infinity, alignment: .leading)

            Text(message)
                .font(OnboardingRecoveryTypography.body)
                .foregroundStyle(.coveLightGray.opacity(0.76))
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

private struct CloudBackupVerificationActions: View {
    let primaryButtonTitle: String
    let isVerifying: Bool
    let verifyAvailability: CloudBackupActionAvailability?
    let verifyOutcome: CloudBackupOutcome?
    let verify: () -> Void
    let dismiss: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            Button(action: verify) {
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
            .disabled(!verifyAvailability.isAvailable)
            .accessibilityIdentifier(CloudBackupScreenAction.verify.accessibilityIdentifier)

            if let reason = verifyAvailability?.unavailableReason {
                Text(reason.caption)
                    .font(.caption)
                    .foregroundStyle(.coveLightGray.opacity(0.76))
                    .accessibilityIdentifier(CloudBackupUiElement.actionUnavailable(action: .verify).identifier)
            }

            if let verifyOutcome {
                Text(verifyOutcome.message)
                    .font(.caption)
                    .foregroundStyle(Color.statusWarning)
                    .accessibilityIdentifier(CloudBackupUiElement.lastOutcome.identifier)
            }

            Button(isVerifying ? "Hide" : "Not Now", action: dismiss)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

private struct CloudBackupVerificationBackground: View {
    let heroTint: Color

    var body: some View {
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
