//
//  CoveMainView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct CoveMainView: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.scenePhase) private var phase

    @State var app: AppManager
    @State var auth: AuthManager

    @State var id = UUID()
    @State var showCover: Bool = true
    @State var scannedCode: TaggedItem<MultiFormat>? = .none
    @State var coverClearTask: Task<Void, Never>?

    private var presentationContext: CoveMainPresentationContext {
        CoveMainPresentationContext(app: app, scannedCode: $scannedCode)
    }

    func onChangeRoute(_ old: [Route], _ new: [Route]) {
        // defer view identity reset to avoid ChildEnvironment propagation loop
        // during UIKit's parallax transition animation
        if !old.isEmpty, new.isEmpty {
            DispatchQueue.main.async { id = UUID() }
        }

        app.reconcileRouteOwnedManagers()
        app.dispatch(action: AppAction.updateRoute(routes: new))
    }

    func onChangeQr(
        _: TaggedItem<MultiFormat>?, _ scannedCode: TaggedItem<MultiFormat>?
    ) {
        Log.debug("[COVE APP ROOT] onChangeQr")
        guard let scannedCode else { return }
        app.sheetState = .none
        ScanManager.shared.handleMultiFormat(scannedCode.item)
    }

    func onChangeNfc(_: NfcMessage?, _ nfcMessage: NfcMessage?) {
        Log.debug("[COVE APP ROOT] onChangeNfc")
        guard let nfcMessage else { return }
        ScanManager.shared.handleNfcScan(nfcMessage)
    }

    func handleScenePhaseChange(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        Log.debug(
            "[SCENE PHASE]: \(oldPhase) --> \(newPhase) && using biometrics: \(auth.isUsingBiometrics)"
        )

        if oldPhase == .active, newPhase != .active {
            app.hideSensitiveKeyTeleportReveals()

            // an NFC scan makes the app inactive, covering it would interrupt the scan
            let tapSignerScanning = app.tapSignerNfc?.isScanning ?? false
            if !app.nfcWriter.isScanning, !app.nfcReader.isScanning, !tapSignerScanning {
                showCover = true
            }
        }

        unlockWhenAuthenticationIsDisabled()
        guard handleActivePhase(newPhase) else { return }

        handleLeavingActive(oldPhase, newPhase)
        handleBackgroundPhase(newPhase)
        handleReturningToActive(oldPhase, newPhase)
        leaveDecoyModeIfNeeded(newPhase)
    }

    private func unlockWhenAuthenticationIsDisabled() {
        guard !auth.isAuthEnabled else { return }

        auth.unlock()
    }

    private func handleActivePhase(_ newPhase: ScenePhase) -> Bool {
        guard newPhase == .active else { return true }

        showCover = false
        app.endInitialScanBackgroundTask()
        guard app.asyncRuntimeReady else { return false }

        app.dispatch(action: AppAction.updateFees)
        app.dispatch(action: AppAction.updateFiatPrices)

        return true
    }

    private func handleLeavingActive(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        guard auth.isAuthEnabled else { return }
        guard !auth.isUsingBiometrics else { return }
        guard oldPhase == .active, newPhase == .inactive else { return }

        Log.debug("[scene] app going inactive")
        coverClearTask?.cancel()

        if !isNfcOperationActive {
            showCover = true
        }

        scheduleCoverRecovery()
    }

    private var isNfcOperationActive: Bool {
        let tapSignerScanning = app.tapSignerNfc?.isScanning ?? false
        return app.nfcWriter.isScanning || app.nfcReader.isScanning || tapSignerScanning
    }

    private func scheduleCoverRecovery() {
        coverClearTask = Task {
            try? await Task.sleep(for: .milliseconds(100))
            guard !Task.isCancelled else { return }

            if phase == .active { showCover = false }

            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }

            if phase == .active { showCover = false }
        }
    }

    private func handleBackgroundPhase(_ newPhase: ScenePhase) {
        guard newPhase == .background else { return }

        app.isSidebarVisible = false
        app.beginInitialScanBackgroundTaskIfNeeded()
        guard auth.isAuthEnabled else { return }

        Log.debug("[scene] app going into background")
        coverClearTask?.cancel()

        guard !isNfcOperationActive else {
            Log.debug("[scene] NFC operation active, not dismissing sheets or locking")
            return
        }

        showCover = true
        if auth.lockState != .locked { auth.lock() }

        dismissPresentedViewControllers()
        UIApplication.shared.endEditing()
    }

    private func dismissPresentedViewControllers() {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .forEach { window in
                window.rootViewController?.dismiss(animated: false)
            }
    }

    private func handleReturningToActive(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        guard auth.isAuthEnabled, oldPhase == .inactive, newPhase == .active else { return }
        guard let lockedAt = auth.lockedAt else { return }

        let sinceLocked = Date.now.timeIntervalSince(lockedAt)
        Log.debug("[ROOT][AUTH] lockedAt \(lockedAt) == \(sinceLocked)")

        // TODO: make this configurable and put in DB
        if auth.type == .pin, !auth.isDecoyPinEnabled, sinceLocked < 2 {
            showCover = false
            auth.unlock()
            return
        }

        if sinceLocked < 1 {
            showCover = false
            auth.unlock()
        }
    }

    private func leaveDecoyModeIfNeeded(_ newPhase: ScenePhase) {
        guard auth.isInDecoyMode(), newPhase == .active else { return }
        guard auth.type == .none || auth.type == .biometric else { return }

        auth.switchToMainMode()
    }

    private func navigate(to route: Route) {
        app.pushRoute(route)
    }

    private func resetViewIdentity() {
        id = UUID()
    }

    var body: some View {
        CloudBackupPresentationHost(app: app, auth: auth, isCoverPresented: showCover) {
            CoveMainPresentedContent(
                app: app,
                auth: auth,
                showCover: $showCover,
                scannedCode: $scannedCode,
                id: id,
                phase: phase,
                presentationContext: presentationContext,
                navigate: navigate,
                resetIdentity: resetViewIdentity,
                onChangeRoute: onChangeRoute,
                onChangeQr: onChangeQr,
                onChangeNfc: onChangeNfc,
                onChangePhase: handleScenePhaseChange
            )
        }
    }
}

private struct CoveMainPresentedContent: View {
    @Bindable var app: AppManager
    let auth: AuthManager
    @Binding var showCover: Bool
    @Binding var scannedCode: TaggedItem<MultiFormat>?
    let id: UUID
    let phase: ScenePhase
    let presentationContext: CoveMainPresentationContext
    let navigate: (Route) -> Void
    let resetIdentity: () -> Void
    let onChangeRoute: ([Route], [Route]) -> Void
    let onChangeQr: (TaggedItem<MultiFormat>?, TaggedItem<MultiFormat>?) -> Void
    let onChangeNfc: (NfcMessage?, NfcMessage?) -> Void
    let onChangePhase: (ScenePhase, ScenePhase) -> Void

    var body: some View {
        CoveLockedContent(app: app, auth: auth, showCover: $showCover)
            .id(id)
            .environment(\.navigate, navigate)
            .environment(app)
            .preferredColorScheme(app.colorScheme)
            .onChange(of: app.router.routes, onChangeRoute)
            .onChange(of: app.selectedNetwork) { resetIdentity() }
            .onChange(of: scannedCode, onChangeQr)
            .onChange(of: app.nfcReader.scannedMessage, onChangeNfc)
            .presentingAlert($app.alertState, context: presentationContext)
            .presentingSheet($app.sheetState, context: presentationContext)
            .onOpenURL(perform: ScanManager.shared.handleFileOpen)
            .onChange(of: phase, initial: true, onChangePhase)
    }
}

private struct CoveLockedContent: View {
    @Bindable var app: AppManager
    @Bindable var auth: AuthManager
    @Binding var showCover: Bool

    var body: some View {
        LockView(
            lockType: auth.type,
            isPinCorrect: isPinCorrect,
            showPin: false,
            lockState: $auth.lockState,
            onUnlock: handleUnlock
        ) {
            CoveNavigationContent(app: app)
        }
        .fullScreenCover(isPresented: $app.isLoading) {
            FullPageLoadingView().interactiveDismissDisabled(true)
        }
        .fullScreenCover(isPresented: $showCover) {
            CoverView().interactiveDismissDisabled(true)
        }
        .onChange(of: auth.lockState) { old, new in
            Log.warn("AUTH LOCK STATE CHANGED: \(old) --> \(new)")
        }
        .overlay {
            if auth.wipePresentationState == .running {
                ZStack {
                    Color.black.opacity(0.8).ignoresSafeArea()
                    ProgressView("Removing local wallet data…")
                        .tint(.white)
                        .foregroundStyle(.white)
                }
            }
        }
        .alert(
            "Wallet Shutdown Is Blocked",
            isPresented: Binding(
                get: { blockedAttempt != nil },
                set: { _ in }
            )
        ) {
            if let attemptId = blockedAttempt {
                Button("Retry") {
                    Task { await auth.retryWipe(attemptId) }
                }
                Button("Cancel", role: .cancel) {
                    auth.cancelWipe(attemptId)
                }
            }
        } message: {
            Text("Cove could not stop all wallet work. Retry or cancel the wipe.")
        }
        .alert(
            "Unable to Remove Local Data",
            isPresented: Binding(
                get: {
                    if case .failed = auth.wipePresentationState { true } else { false }
                },
                set: { presented in
                    if !presented { auth.clearWipeFailure() }
                }
            )
        ) {
            Button("OK", role: .cancel) { auth.clearWipeFailure() }
        } message: {
            if case let .failed(message) = auth.wipePresentationState {
                Text(message)
            }
        }
        .environment(app)
        .environment(auth)
    }

    private var blockedAttempt: ShutdownAttemptId? {
        if case let .shutdownBlocked(attemptId) = auth.wipePresentationState {
            return attemptId
        }

        return nil
    }

    private func isPinCorrect(_ pin: String) async -> Bool {
        await auth.handleAndReturnUnlockMode(pin) != .locked
    }

    private func handleUnlock(_: String) {
        withAnimation {
            showCover = false
        }
    }
}

private struct CoveNavigationContent: View {
    @Bindable var app: AppManager

    var body: some View {
        SidebarContainer {
            NavigationStack(path: $app.router.routes) {
                RouteView(app: app)
                    .navigationDestination(for: Route.self) { route in
                        RouteView(app: app, route: route)
                    }
                    .modifier(CoveSidebarToolbarModifier(app: app))
            }
            .modifier(ConditionalRouteTintModifier(route: app.router.routes.last))
        }
    }
}

private struct CoveSidebarToolbarModifier: ViewModifier {
    let app: AppManager

    func body(content: Content) -> some View {
        content.toolbar {
            ToolbarItem(placement: .navigationBarLeading) {
                CoveSidebarButton(app: app)
            }
        }
    }
}

private struct CoveSidebarButton: View {
    let app: AppManager

    var body: some View {
        Button {
            withAnimation {
                app.toggleSidebar()
            }
        } label: {
            Image(systemName: "line.horizontal.3")
                .modifier(
                    NavBarColorModifier(
                        route: app.currentRoute,
                        isPastHeader: app.isPastHeader
                    )
                )
        }
        .contentShape(Rectangle())
        .accessibilityIdentifier("app.sidebar.open")
    }
}
