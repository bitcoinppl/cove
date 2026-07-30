import SwiftUI

struct KeyTeleportContainer: View {
    @Environment(AppManager.self) private var app

    let route: KeyTeleportRoute

    @State private var scannedCode: TaggedItem<MultiFormat>?
    @State private var manager: KeyTeleportManager?

    var body: some View {
        Group {
            if let manager {
                KeyTeleportLoadedView(
                    manager: manager,
                    route: route,
                    scannedCode: $scannedCode
                )
            } else {
                ProgressView()
                    .task {
                        manager = app.ensureKeyTeleportManager()
                    }
            }
        }
        .environment(app)
    }
}

private struct KeyTeleportLoadedView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.scenePhase) private var scenePhase

    @Bindable var manager: KeyTeleportManager
    let route: KeyTeleportRoute
    @Binding var scannedCode: TaggedItem<MultiFormat>?

    @State private var showScanner = false
    @State private var pastedText = ""
    @State private var receiverCode = ""
    @State private var senderPassword = ""
    @State private var mnemonicDisclosure = KeyTeleportMnemonicDisclosure.hidden
    @State private var xprv: String?
    @State private var sensitiveRevealResetId = UUID()
    @State private var showEndSessionConfirmation = false
    @State private var showRestartSessionConfirmation = false

    var body: some View {
        KeyTeleportLoadedContent(
            manager: manager,
            route: route,
            pastedText: $pastedText,
            receiverCode: $receiverCode,
            senderPassword: $senderPassword,
            mnemonicDisclosure: $mnemonicDisclosure,
            xprv: $xprv,
            sensitiveRevealResetId: sensitiveRevealResetId,
            scan: openScanner,
            paste: paste,
            revealMnemonicWords: revealMnemonicWords,
            finishReview: finishReview
        )
        .scrollDismissesKeyboard(.interactively)
        .foregroundStyle(.white)
        .tint(.btnGradientLight)
        .onboardingRecoveryBackground()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            KeyTeleportReadyActionsToolbar(
                manager: manager,
                showEndSessionConfirmation: $showEndSessionConfirmation,
                showRestartSessionConfirmation: $showRestartSessionConfirmation
            )
        }
        .sheet(isPresented: $showScanner) {
            QrCodeScanView(app: app, scannedCode: $scannedCode)
        }
        .onAppear(perform: prepare)
        .onDisappear(perform: handleDisappear)
        .onChange(of: scenePhase, handleScenePhaseChange)
        .onChange(of: isMnemonicReview, handleMnemonicReviewChange)
        .onChange(of: isXprvReview, handleXprvReviewChange)
        .onChange(of: scannedCode, handleScannedCodeChange)
    }

    private func prepare() {
        if case .receive = route, case .idle = manager.state {
            manager.dispatch(.startReceive)
        }
    }

    private func handleDisappear() {
        resetSensitiveRevealUI()
    }

    private func resetSensitiveRevealUI() {
        senderPassword = ""
        receiverCode = ""
        mnemonicDisclosure = .hidden
        xprv = nil
        sensitiveRevealResetId = UUID()

        manager.hideSensitiveReveals()
    }

    private func openScanner() {
        showScanner = true
    }

    private func handleScenePhaseChange(_ oldPhase: ScenePhase, _ newPhase: ScenePhase) {
        guard oldPhase == .active, newPhase != .active else { return }

        resetSensitiveRevealUI()
    }

    private func handleMnemonicReviewChange(_: Bool, _ isReview: Bool) {
        if !isReview {
            mnemonicDisclosure = .hidden
        }
    }

    private func handleXprvReviewChange(_: Bool, _ isReview: Bool) {
        if !isReview {
            xprv = nil
        }
    }

    private func handleScannedCodeChange(
        _: TaggedItem<MultiFormat>?,
        _ scannedCode: TaggedItem<MultiFormat>?
    ) {
        guard let multiFormat = scannedCode?.item else { return }

        ingest(multiFormat)
        self.scannedCode = nil
    }

    private func paste() {
        let text = pastedText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }

        do {
            try ingest(StringOrData(text).toMultiFormat())
        } catch {
            manager.ingest(text)
        }
    }

    private func revealMnemonicWords() {
        guard case .hidden = mnemonicDisclosure else { return }

        let words = manager.revealMnemonicWords()
        mnemonicDisclosure = words.isEmpty ? .failed : .revealed(words)
    }

    private func finishReview() {
        manager.dispatch(.finishReview)
        app.popRoute()
    }

    private func ingest(_ multiFormat: MultiFormat) {
        switch multiFormat {
        case let .keyTeleportReceiver(packet):
            guard canIngestPacket(requiring: .send) else {
                showDirectionConflict(requiredDirection: .send)
                return
            }

            manager.ingest(packet)
        case let .keyTeleportSender(packet):
            guard canIngestPacket(requiring: .receive) else {
                showDirectionConflict(requiredDirection: .receive)
                return
            }

            manager.ingest(packet)
        default:
            app.alertState = .init(.invalidFormat(message: "This is not a KeyTeleport packet."))
        }
    }

    private func canIngestPacket(requiring requiredDirection: KeyTeleportFlowDirection) -> Bool {
        KeyTeleportPacketRoutingDecision.resolve(
            activeDirections: [route.flowDirection, manager.flowDirection].compactMap(\.self),
            requiredDirection: requiredDirection
        ) == .accept
    }

    private func showDirectionConflict(requiredDirection: KeyTeleportFlowDirection) {
        let expectedPacket = requiredDirection == .send ? "receiver request" : "sender response"
        app.alertState = .init(
            .general(
                title: "Wrong KeyTeleport Packet",
                message: "This session expects a \(expectedPacket). End it before starting the opposite flow."
            )
        )
    }

    private var isMnemonicReview: Bool {
        if case .receiveMnemonicReview = manager.state { return true }
        return false
    }

    private var isXprvReview: Bool {
        if case .receiveXprvReview = manager.state { return true }
        return false
    }
}

private struct KeyTeleportLoadedContent: View {
    @Bindable var manager: KeyTeleportManager
    let route: KeyTeleportRoute
    @Binding var pastedText: String
    @Binding var receiverCode: String
    @Binding var senderPassword: String
    @Binding var mnemonicDisclosure: KeyTeleportMnemonicDisclosure
    @Binding var xprv: String?
    let sensitiveRevealResetId: UUID
    let scan: () -> Void
    let paste: () -> Void
    let revealMnemonicWords: () -> Void
    let finishReview: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                KeyTeleportHeader(route: route)

                if let alert = manager.alert {
                    KeyTeleportAlertBanner(alert: alert, dismiss: dismissAlert)
                }

                KeyTeleportStateContent(
                    manager: manager,
                    route: route,
                    pastedText: $pastedText,
                    receiverCode: $receiverCode,
                    senderPassword: $senderPassword,
                    mnemonicDisclosure: $mnemonicDisclosure,
                    xprv: $xprv,
                    scan: scan,
                    paste: paste,
                    revealMnemonicWords: revealMnemonicWords,
                    finishReview: finishReview
                )
                .id(sensitiveRevealResetId)
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 24)
        }
    }

    private func dismissAlert() {
        manager.alert = nil
    }
}
