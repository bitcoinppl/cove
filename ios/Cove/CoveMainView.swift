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

    var navBarColor: Color {
        switch app.currentRoute {
        case .newWallet(.hotWallet(.create)):
            Color.white
        case .newWallet(.hotWallet(.verifyWords)):
            Color.white
        case .selectedWallet:
            Color.white
        default:
            Color.blue
        }
    }

    private var presentationContext: CoveMainPresentationContext {
        CoveMainPresentationContext(app: app, scannedCode: $scannedCode)
    }

    var BodyView: some View {
        LockView(
            lockType: auth.type,
            isPinCorrect: { pin in
                auth.handleAndReturnUnlockMode(pin) != .locked
            },
            showPin: false,
            lockState: $auth.lockState,
            onUnlock: { _ in
                withAnimation { showCover = false }
            }
        ) {
            SidebarContainer {
                NavigationStack(path: $app.router.routes) {
                    RouteView(app: app)
                        .navigationDestination(
                            for: Route.self,
                            destination: { route in
                                RouteView(app: app, route: route)
                            }
                        )
                        .toolbar {
                            ToolbarItem(placement: .navigationBarLeading) {
                                Button(action: {
                                    withAnimation {
                                        app.toggleSidebar()
                                    }
                                }) {
                                    Image(systemName: "line.horizontal.3")
                                        .modifier(
                                            NavBarColorModifier(
                                                route: app.currentRoute,
                                                isPastHeader: app.isPastHeader
                                            )
                                        )
                                }
                                .contentShape(Rectangle())
                            }
                        }
                }
                .modifier(ConditionalRouteTintModifier(route: app.router.routes.last))
            }
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
        .environment(app)
        .environment(auth)
    }

    func onChangeRoute(_ old: [Route], _ new: [Route]) {
        // defer view identity reset to avoid ChildEnvironment propagation loop
        // during UIKit's parallax transition animation
        if !old.isEmpty, new.isEmpty {
            DispatchQueue.main.async { id = UUID() }
        }

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

        if !auth.isAuthEnabled {
            showCover = false
            auth.unlock()
        }

        if newPhase == .active {
            showCover = false
            app.endInitialScanBackgroundTask()
            guard app.asyncRuntimeReady else { return }
            app.dispatch(action: AppAction.updateFees)
            app.dispatch(action: AppAction.updateFiatPrices)
        }

        // PIN auth active, no biometrics, leaving app
        if auth.isAuthEnabled,
           !auth.isUsingBiometrics,
           oldPhase == .active,
           newPhase == .inactive
        {
            Log.debug("[scene] app going inactive")
            coverClearTask?.cancel()

            let tapSignerScanning = app.tapSignerNfc?.isScanning ?? false
            if !app.nfcWriter.isScanning, !app.nfcReader.isScanning, !tapSignerScanning {
                showCover = true
            }

            // prevent getting stuck on show cover
            coverClearTask = Task {
                try? await Task.sleep(for: .milliseconds(100))
                if Task.isCancelled { return }

                if phase == .active { showCover = false }

                try? await Task.sleep(for: .milliseconds(200))
                if Task.isCancelled { return }

                if phase == .active { showCover = false }
            }
        }

        if newPhase == .background {
            app.isSidebarVisible = false
            app.beginInitialScanBackgroundTaskIfNeeded()
        }

        // close all open sheets when going into the background
        if auth.isAuthEnabled, newPhase == .background {
            Log.debug("[scene] app going into background")
            coverClearTask?.cancel()

            // don't lock or dismiss sheets if any NFC operation is active
            let tapSignerScanning = app.tapSignerNfc?.isScanning ?? false
            if app.nfcWriter.isScanning || app.nfcReader.isScanning || tapSignerScanning {
                Log.debug("[scene] NFC operation active, not dismissing sheets or locking")
                return
            }

            showCover = true
            if auth.lockState != .locked { auth.lock() }

            UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows)
                .forEach { window in
                    window.rootViewController?.dismiss(animated: false)
                }

            // dismiss all keyboard
            UIApplication.shared.endEditing()
        }

        // auth enabled, opening app again
        if auth.isAuthEnabled, oldPhase == .inactive, newPhase == .active {
            guard let lockedAt = auth.lockedAt else { return }
            let sinceLocked = Date.now.timeIntervalSince(lockedAt)
            Log.debug("[ROOT][AUTH] lockedAt \(lockedAt) == \(sinceLocked)")

            // less than 1 second, auto unlock if PIN only, and not in decoy mode
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

        // sanity check, get out of decoy mode if PIN is disabled
        if auth.isInDecoyMode(), newPhase == .active,
           auth.type == .none || auth.type == .biometric
        {
            auth.switchToMainMode()
        }
    }

    var body: some View {
        CloudBackupPresentationHost(app: app, auth: auth, isCoverPresented: showCover) {
            BodyView
                .id(id)
                .environment(\.navigate) { route in
                    app.pushRoute(route)
                }
                .environment(app)
                .preferredColorScheme(app.colorScheme)
                .onChange(of: app.router.routes, onChangeRoute)
                .onChange(of: app.selectedNetwork) { id = UUID() }
                .onChange(of: scannedCode, onChangeQr)
                .onChange(of: app.nfcReader.scannedMessage, onChangeNfc)
                .presentingAlert($app.alertState, context: presentationContext)
                .presentingSheet($app.sheetState, context: presentationContext)
                .onOpenURL(perform: ScanManager.shared.handleFileOpen)
                .onChange(of: phase, initial: true, handleScenePhaseChange)
        }
    }
}
