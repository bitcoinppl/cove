import SwiftUI
import UIKit

struct KeyTeleportContainer: View {
    @Environment(AppManager.self) private var app

    let route: KeyTeleportRoute

    @State private var scannedCode: TaggedItem<MultiFormat>?

    var body: some View {
        KeyTeleportLoadedView(
            manager: app.ensureKeyTeleportManager(),
            route: route,
            scannedCode: $scannedCode
        )
        .environment(app)
    }
}

private struct KeyTeleportLoadedView: View {
    @Environment(AppManager.self) private var app

    @Bindable var manager: KeyTeleportManager
    let route: KeyTeleportRoute
    @Binding var scannedCode: TaggedItem<MultiFormat>?

    @State private var showScanner = false
    @State private var pastedText = ""
    @State private var receiverCode = ""
    @State private var senderPassword = ""
    @State private var xprv: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header

                if let alert = manager.alert {
                    KeyTeleportAlertBanner(alert: alert)
                }

                stateContent
            }
            .padding()
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $showScanner) {
            QrCodeScanView(app: app, scannedCode: $scannedCode)
        }
        .onAppear(perform: prepare)
        .onDisappear(perform: handleDisappear)
        .onChange(of: scannedCode) { _, scannedCode in
            guard let multiFormat = scannedCode?.item else { return }

            ingest(multiFormat)
            self.scannedCode = nil
        }
    }

    @ViewBuilder
    private var stateContent: some View {
        switch manager.state {
        case .idle:
            KeyTeleportScanPasteSection(
                pastedText: $pastedText,
                scan: { showScanner = true },
                paste: paste
            )
        case let .receiveReplacementRequired(state):
            KeyTeleportReceiveReadyView(state: state, replacementRequired: true) {
                manager.dispatch(.confirmReplaceReceive)
            } scan: {
                showScanner = true
            } share: {
                ShareSheet.present(text: state.packet.url())
            }
        case let .receiveReady(state):
            KeyTeleportReceiveReadyView(state: state, replacementRequired: false) {
                manager.dispatch(.confirmReplaceReceive)
            } scan: {
                showScanner = true
            } share: {
                ShareSheet.present(text: state.packet.url())
            }
        case .receiveEnterPassword:
            KeyTeleportPasswordEntryView(password: $senderPassword) {
                manager.dispatch(.enterSenderPassword(senderPassword))
            }
        case let .receiveMnemonicReview(review):
            KeyTeleportMnemonicReviewView(review: review, words: manager.revealMnemonicWords()) {
                manager.dispatch(.importReceivedMnemonic)
            } finish: {
                manager.dispatch(.finishReview)
                app.popRoute()
            }
        case let .receiveXprvReview(review):
            KeyTeleportXprvReviewView(review: review, xprv: $xprv) {
                xprv = manager.revealXprv()
            } hide: {
                xprv = nil
                manager.dispatch(.hideXprv)
            } finish: {
                xprv = nil
                manager.dispatch(.finishReview)
                app.popRoute()
            }
        case let .sendChooseWallet(state):
            KeyTeleportSendChooseWalletView(state: state) { walletId in
                manager.dispatch(.selectSendWallet(walletId))
            }

            KeyTeleportScanPasteSection(
                pastedText: $pastedText,
                scan: { showScanner = true },
                paste: paste
            )
        case let .sendEnterCode(state):
            KeyTeleportReceiverCodeView(state: state, code: $receiverCode) {
                manager.dispatch(.enterReceiverCode(receiverCode))
            }
        case let .sendConfirm(state):
            KeyTeleportSendConfirmView(state: state) {
                manager.dispatch(.confirmSendMnemonic)
            }
        case let .sendReady(state):
            KeyTeleportSendReadyView(state: state) {
                ShareSheet.present(text: state.packet.url())
            } finish: {
                manager.dispatch(.clear)
                app.popRoute()
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.title2)
                .fontWeight(.semibold)

            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
    }

    private var title: String {
        switch route {
        case .receive:
            "Receive by Key Teleport"
        case .send:
            "Send by Key Teleport"
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

    private func prepare() {
        if case .receive = route, case .idle = manager.state {
            manager.dispatch(.startReceive)
        }
    }

    private func handleDisappear() {
        if case .receiveXprvReview = manager.state {
            xprv = nil
            manager.dispatch(.hideXprv)
        }
    }

    private func paste() {
        let text = pastedText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }

        manager.ingest(text)
    }

    private func ingest(_ multiFormat: MultiFormat) {
        switch multiFormat {
        case let .keyTeleportReceiver(packet):
            manager.ingest(packet.bbqrPart())
        case let .keyTeleportSender(packet):
            manager.ingest(packet.bbqrPart())
        default:
            app.alertState = .init(.invalidFormat(message: "This is not a Key Teleport packet."))
        }
    }
}

private struct KeyTeleportAlertBanner: View {
    let alert: KeyTeleportAlert

    var body: some View {
        Text(message)
            .font(.subheadline)
            .foregroundStyle(.red)
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
        case .ParseFailed:
            "This Key Teleport packet could not be read."
        case .UnsupportedPsbt:
            "Key Teleport PSBT packets are not supported yet."
        case .WrongReceiverCode:
            "The receiver code does not match this request."
        case .WrongTeleportPassword:
            "The Teleport Password is incorrect."
        case .NoEligibleWallets:
            "No wallet on this device can send with Key Teleport."
        case .IneligibleWallet:
            "This wallet cannot send with Key Teleport."
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

private struct KeyTeleportScanPasteSection: View {
    @Binding var pastedText: String
    let scan: () -> Void
    let paste: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Button(action: scan) {
                Label("Scan QR", systemImage: "qrcode.viewfinder")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)

            TextField("Paste Key Teleport packet or link", text: $pastedText, axis: .vertical)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(3, reservesSpace: true)
                .textFieldStyle(.roundedBorder)

            Button(action: paste) {
                Label("Use Pasted Packet", systemImage: "doc.on.clipboard")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
        }
    }
}

private struct KeyTeleportReceiveReadyView: View {
    let state: KeyTeleportReceiveState
    let replacementRequired: Bool
    let replace: () -> Void
    let scan: () -> Void
    let share: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            if replacementRequired {
                Text("An active receive session already exists.")
                    .font(.subheadline)
                    .foregroundStyle(.orange)
            }

            QrCodeView(text: state.packet.bbqrPart())
                .frame(maxWidth: 280)
                .frame(maxWidth: .infinity)

            VStack(spacing: 4) {
                Text("Receiver Code")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text(state.groupedNumericCode)
                    .font(.system(.title2, design: .monospaced))
                    .fontWeight(.semibold)
            }

            HStack {
                Button { UIPasteboard.general.string = state.packet.url() } label: {
                    Label("Copy Link", systemImage: "doc.on.doc")
                }
                .buttonStyle(.bordered)

                Button(action: share) {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                .buttonStyle(.bordered)
            }

            Button(action: scan) {
                Label("Scan Sender Response", systemImage: "qrcode.viewfinder")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)

            if replacementRequired {
                Button(role: .destructive, action: replace) {
                    Text("Replace Receive Session")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
            }
        }
    }
}

private struct KeyTeleportPasswordEntryView: View {
    @Binding var password: String
    let submit: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SecureField("Teleport Password", text: $password)
                .textInputAutocapitalization(.characters)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)

            Button(action: submit) {
                Text("Unlock")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
        }
    }
}

private struct KeyTeleportMnemonicReviewView: View {
    let review: KeyTeleportMnemonicReview
    let words: [String]
    let importWords: () -> Void
    let finish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let wallet = review.importedWallet {
                Text("Imported \(wallet.name)")
                    .font(.headline)
                Button("Done", action: finish)
                    .buttonStyle(.borderedProminent)
            } else {
                Text("\(review.wordCount)-word wallet")
                    .font(.headline)

                LazyVGrid(columns: [GridItem(.adaptive(minimum: 120), spacing: 8)], spacing: 8) {
                    ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                        HStack {
                            Text("\(index + 1)")
                                .foregroundStyle(.secondary)
                            Text(word)
                            Spacer()
                        }
                        .font(.system(.subheadline, design: .monospaced))
                        .padding(8)
                        .background(Color.secondary.opacity(0.10))
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                    }
                }

                Button(action: importWords) {
                    Text("Import Wallet")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)

                Button("Finish Without Importing", role: .destructive, action: finish)
                    .buttonStyle(.bordered)
            }
        }
    }
}

private struct KeyTeleportXprvReviewView: View {
    let review: KeyTeleportXprvReview
    @Binding var xprv: String?
    let reveal: () -> Void
    let hide: () -> Void
    let finish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Extended private key")
                .font(.headline)

            if review.revealed, let xprv {
                Text(xprv)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .padding()
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.secondary.opacity(0.10))
                    .clipShape(RoundedRectangle(cornerRadius: 8))

                HStack {
                    Button { UIPasteboard.general.string = xprv } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                    .buttonStyle(.bordered)

                    Button("Hide", action: hide)
                        .buttonStyle(.bordered)
                }
            } else {
                Text("Reveal only if you are ready to handle this private key.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)

                Button(action: reveal) {
                    Text("Reveal XPRV")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
            }

            Button("Finish", action: finish)
                .buttonStyle(.bordered)
        }
    }
}

private struct KeyTeleportSendChooseWalletView: View {
    let state: KeyTeleportSendChooseWallet
    let select: (WalletId) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Wallet")
                .font(.headline)

            ForEach(state.eligibleWallets, id: \.id) { wallet in
                Button {
                    select(wallet.id)
                } label: {
                    HStack {
                        Text(wallet.name)
                        Spacer()
                        if state.selectedWallet == wallet.id {
                            Image(systemName: "checkmark")
                        }
                    }
                }
                .buttonStyle(.bordered)
            }
        }
    }
}

private struct KeyTeleportReceiverCodeView: View {
    let state: KeyTeleportSendEnterCode
    @Binding var code: String
    let submit: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Sending from \(state.selectedWallet.name)")
                .font(.headline)

            TextField("Receiver Code", text: $code)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)

            Button(action: submit) {
                Text("Continue")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
        }
    }
}

private struct KeyTeleportSendConfirmView: View {
    let state: KeyTeleportSendConfirm
    let confirm: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Send \(state.selectedWallet.name)")
                .font(.headline)

            if state.warnsPassphraseNotIncluded {
                Text("Only the wallet words will be sent. Any BIP39 passphrase is not included.")
                    .font(.subheadline)
                    .foregroundStyle(.orange)
            }

            Button(action: confirm) {
                Text("Create Sender Response")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
        }
    }
}

private struct KeyTeleportSendReadyView: View {
    let state: KeyTeleportSendReady
    let share: () -> Void
    let finish: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            QrCodeView(text: state.packet.bbqrPart())
                .frame(maxWidth: 280)
                .frame(maxWidth: .infinity)

            VStack(spacing: 4) {
                Text("Teleport Password")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text(state.password.groupedText())
                    .font(.system(.title2, design: .monospaced))
                    .fontWeight(.semibold)
            }

            HStack {
                Button { UIPasteboard.general.string = state.packet.url() } label: {
                    Label("Copy Link", systemImage: "doc.on.doc")
                }
                .buttonStyle(.bordered)

                Button(action: share) {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                .buttonStyle(.bordered)
            }

            Button("Done", action: finish)
                .buttonStyle(.borderedProminent)
        }
    }
}
