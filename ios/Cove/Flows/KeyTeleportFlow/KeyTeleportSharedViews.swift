import SwiftUI

struct KeyTeleportHeader: View {
    let route: KeyTeleportRoute

    private var title: String {
        switch route {
        case .receive:
            "Receive by KeyTeleport"
        case .send:
            "Send by KeyTeleport"
        }
    }

    private var subtitle: String {
        switch route {
        case .receive:
            "Show this request to the sending wallet, then scan the sender response."
        case .send:
            "Scan or paste the receiver request, confirm the wallet, then share the response."
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(OnboardingRecoveryTypography.compactTitle)

            Text(subtitle)
                .font(OnboardingRecoveryTypography.footnote)
                .foregroundStyle(.coveLightGray.opacity(0.74))
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct KeyTeleportAlertBanner: View {
    let alert: KeyTeleportAlert
    let dismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Text(message)
                .font(.subheadline)
                .foregroundStyle(.red)
                .frame(maxWidth: .infinity, alignment: .leading)

            Button(action: dismiss) {
                Image(systemName: "xmark.circle.fill")
                    .imageScale(.medium)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.red)
            .accessibilityLabel("Dismiss")
        }
        .padding()
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.red.opacity(0.12))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var message: String {
        switch alert {
        case .NoActiveReceiveSession:
            "Start a receive session before scanning a sender response."
        case .ReceiveSessionExpired:
            "This receive session expired. Start a new receive session."
        case .ReceiveSessionReset:
            "The previous receive request was unreadable, so Cove replaced it. Responses for the old request will not work."
        case .ReceiveSessionScopeChanged:
            "Return to the network and wallet mode where this receive session was started."
        case .ConflictingTransferDirection:
            "Finish the active KeyTeleport transfer before starting the opposite direction."
        case .ParseFailed:
            "This KeyTeleport packet could not be read."
        case .UnsupportedPsbt:
            "KeyTeleport PSBT packets are not supported yet."
        case .UnsupportedPayload:
            "This type of KeyTeleport payload is not supported yet."
        case .InvalidPayload:
            "The transfer was unlocked, but its contents are not valid KeyTeleport data."
        case .WrongReceiverCode:
            "The receiver code does not match this request."
        case .WrongTeleportPassword:
            "The Teleport Password is incorrect."
        case .NoEligibleWallets:
            "No wallet on this device can send with KeyTeleport."
        case .IneligibleWallet:
            "This wallet cannot send with KeyTeleport."
        case .NoPendingSend:
            "Scan or paste a receiver request first."
        case .NoPendingReceiveSecret:
            "Scan a sender response first."
        case let .ImportFailed(message),
             let .Keychain(message),
             let .Protocol(message),
             let .Database(message):
            message
        }
    }
}

struct KeyTeleportLoadingView: View {
    var body: some View {
        VStack(spacing: 12) {
            ProgressView()

            Text("Preparing receive request...")
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 36)
    }
}

struct KeyTeleportReadyActionsToolbar: ToolbarContent {
    @Environment(AppManager.self) private var app

    @Bindable var manager: KeyTeleportManager
    @Binding var showEndSessionConfirmation: Bool
    @Binding var showRestartSessionConfirmation: Bool

    private var showsReadyActions: Bool {
        switch manager.state {
        case .receiveReady, .sendReady:
            true
        default:
            false
        }
    }

    var body: some ToolbarContent {
        if showsReadyActions {
            ToolbarItem(placement: .navigationBarTrailing) {
                readyActionsMenu
            }
        }
    }

    private var readyActionsMenu: some View {
        Menu {
            switch manager.state {
            case let .receiveReady(state):
                shareButton { try state.packet.url() }

                Button(action: showRestartConfirmation) {
                    Label("New Receive Request", systemImage: "arrow.clockwise")
                }

                Button(
                    role: .destructive,
                    action: showEndConfirmation
                ) {
                    Label("End Session", systemImage: "xmark.circle")
                }
            case let .sendReady(state):
                shareButton { try state.packet.url() }
            default:
                EmptyView()
            }
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .accessibilityLabel("More")
        .confirmationDialog(
            "End this session?",
            isPresented: $showEndSessionConfirmation,
            titleVisibility: .visible
        ) {
            Button("End Session", role: .destructive, action: endSession)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The current receive request will be deleted from this device.")
        }
        .confirmationDialog(
            "Create a new receive request?",
            isPresented: $showRestartSessionConfirmation,
            titleVisibility: .visible
        ) {
            Button("Create New Request", role: .destructive, action: restartSession)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Sender responses made for the current request will no longer work.")
        }
    }

    private func shareButton(url: @escaping () throws -> String) -> some View {
        Button {
            do {
                try ShareSheet.presentFromMenu(text: url())
            } catch {
                app.alertState = TaggedItem(
                    .invalidFormat(message: "Unable to encode this KeyTeleport packet.")
                )
            }
        } label: {
            Label("Share", systemImage: "square.and.arrow.up")
        }
    }

    private func showRestartConfirmation() {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            showRestartSessionConfirmation = true
        }
    }

    private func showEndConfirmation() {
        // wait for menu dismissal so the dialog can anchor to the toolbar button
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            showEndSessionConfirmation = true
        }
    }

    private func endSession() {
        manager.dispatch(.endReceive)
        app.popRoute()
    }

    private func restartSession() {
        manager.dispatch(.restartReceive)
    }
}
