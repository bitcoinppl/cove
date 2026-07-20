import SwiftUI
import UIKit
import UniformTypeIdentifiers

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
    @State private var mnemonicWords: [String]?
    @State private var loadedMnemonicReviewID: KeyTeleportMnemonicReviewID?
    @State private var xprv: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                header

                if let alert = manager.alert {
                    KeyTeleportAlertBanner(alert: alert) {
                        manager.alert = nil
                    }
                }

                stateContent
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 24)
        }
        .scrollDismissesKeyboard(.interactively)
        .foregroundStyle(.white)
        .tint(.btnGradientLight)
        .onboardingRecoveryBackground()
        .navigationTitle("")
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

    private var stateContent: some View {
        VStack(alignment: .leading, spacing: 18) {
            switch manager.state {
            case .idle:
                switch route {
                case .receive:
                    KeyTeleportLoadingView()
                case .send:
                    KeyTeleportScanPasteSection(
                        pastedText: $pastedText,
                        scan: { showScanner = true },
                        paste: paste
                    )
                }
            case let .receiveReady(state):
                KeyTeleportReceiveReadyView(state: state) {
                    showScanner = true
                } share: {
                    ShareSheet.present(text: state.packet.url())
                } restart: {
                    manager.dispatch(.restartReceive)
                } end: {
                    manager.dispatch(.endReceive)
                    app.popRoute()
                }
            case .receiveEnterPassword:
                KeyTeleportPasswordEntryView(password: $senderPassword) {
                    manager.dispatch(.enterSenderPassword(senderPassword))
                }
            case let .receiveMnemonicReview(review):
                KeyTeleportMnemonicReviewView(review: review, words: mnemonicWords) {
                    manager.dispatch(.importReceivedWallet)
                } finish: {
                    mnemonicWords = nil
                    loadedMnemonicReviewID = nil
                    finishReview()
                }
                .protectedFromScreenCapture()
                .task(id: KeyTeleportMnemonicReviewID(review: review)) {
                    loadMnemonicWords(for: review)
                }
            case let .receiveXprvReview(review):
                KeyTeleportXprvReviewView(review: review, xprv: $xprv) {
                    xprv = manager.revealXprv()
                } hide: {
                    xprv = nil
                    manager.dispatch(.hideXprv)
                } importWallet: {
                    manager.dispatch(.importReceivedWallet)
                } finish: {
                    xprv = nil
                    finishReview()
                }
                .protectedFromScreenCapture()
            case let .receiveMessageReview(review):
                KeyTeleportMessageReviewView(review: review, finish: finishReview)
                    .protectedFromScreenCapture()
            case let .receiveImportedWallet(wallet):
                KeyTeleportImportedWalletView(wallet: wallet) {
                    manager.dispatch(.clear)
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
                    manager.dispatch(.confirmSendWallet)
                }
            case let .sendReady(state):
                KeyTeleportSendReadyView(state: state) {
                    ShareSheet.present(text: state.packet.url())
                } finish: {
                    manager.dispatch(.clear)
                    app.popRoute()
                }
                .protectedFromScreenCapture()
            }
        }
        .keyTeleportCard()
    }

    private var header: some View {
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

    private func loadMnemonicWords(for review: KeyTeleportMnemonicReview) {
        let reviewID = KeyTeleportMnemonicReviewID(review: review)
        guard loadedMnemonicReviewID != reviewID else { return }

        mnemonicWords = nil
        mnemonicWords = manager.revealMnemonicWords()
        loadedMnemonicReviewID = reviewID
    }

    private func finishReview() {
        manager.dispatch(.finishReview)
        app.popRoute()
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

private struct KeyTeleportMnemonicReviewID: Equatable {
    let wordCount: UInt32

    init(review: KeyTeleportMnemonicReview) {
        wordCount = review.wordCount
    }
}

private struct KeyTeleportAlertBanner: View {
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
        case .ParseFailed:
            "This Key Teleport packet could not be read."
        case .UnsupportedPsbt:
            "Key Teleport PSBT packets are not supported yet."
        case .UnsupportedPayload:
            "This type of Key Teleport payload is not supported yet."
        case .InvalidPayload:
            "The transfer was unlocked, but its contents are not valid Key Teleport data."
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

private struct KeyTeleportLoadingView: View {
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
            .buttonStyle(OnboardingPrimaryButtonStyle())

            TextField(
                "Paste Key Teleport packet or link",
                text: $pastedText,
                prompt: keyTeleportInputPlaceholder("Paste Key Teleport packet or link"),
                axis: .vertical
            )
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .lineLimit(3, reservesSpace: true)
            .keyTeleportInputChrome()

            Button(action: paste) {
                Label("Use Pasted Packet", systemImage: "doc.on.clipboard")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

private struct KeyTeleportReceiveReadyView: View {
    let state: KeyTeleportReceiveState
    let scan: () -> Void
    let share: () -> Void
    let restart: () -> Void
    let end: () -> Void

    @State private var pendingSessionAction: KeyTeleportReceiveSessionAction?

    var body: some View {
        VStack(spacing: 18) {
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
                .tint(.white)

                Button(action: share) {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                .buttonStyle(.bordered)
                .tint(.white)
            }

            Text("Send the link to another Key Teleport-compatible wallet instead of showing the QR code. Enter the receiver code separately.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            Button(action: scan) {
                Label("Scan Sender Response", systemImage: "qrcode.viewfinder")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())

            HStack {
                Button {
                    pendingSessionAction = .restart
                } label: {
                    Label("New Session", systemImage: "arrow.clockwise")
                }
                .buttonStyle(.bordered)
                .tint(.white)
                .confirmationDialog(
                    "Manage Receive Session",
                    isPresented: isPresentingSessionAction(.restart),
                    titleVisibility: .visible
                ) {
                    Button("Start New Session", role: .destructive) {
                        pendingSessionAction = nil
                        restart()
                    }

                    Button("Cancel", role: .cancel) {
                        pendingSessionAction = nil
                    }
                } message: {
                    Text(KeyTeleportReceiveSessionAction.restart.message)
                }

                Button(role: .destructive) {
                    pendingSessionAction = .end
                } label: {
                    Text("End Session")
                }
                .buttonStyle(.bordered)
                .confirmationDialog(
                    "Manage Receive Session",
                    isPresented: isPresentingSessionAction(.end),
                    titleVisibility: .visible
                ) {
                    Button("End Session", role: .destructive) {
                        pendingSessionAction = nil
                        end()
                    }

                    Button("Cancel", role: .cancel) {
                        pendingSessionAction = nil
                    }
                } message: {
                    Text(KeyTeleportReceiveSessionAction.end.message)
                }
            }
        }
    }

    private func isPresentingSessionAction(_ action: KeyTeleportReceiveSessionAction) -> Binding<Bool> {
        Binding(
            get: { pendingSessionAction == action },
            set: { isPresented in
                if isPresented {
                    pendingSessionAction = action
                } else if pendingSessionAction == action {
                    pendingSessionAction = nil
                }
            }
        )
    }
}

private enum KeyTeleportReceiveSessionAction: Equatable {
    case restart
    case end

    var message: String {
        switch self {
        case .restart:
            "The current link, QR code, and receiver code will stop working."
        case .end:
            "The current receive request will be deleted from this device."
        }
    }
}

private struct KeyTeleportPasswordEntryView: View {
    @Binding var password: String
    let submit: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            KeyTeleportSecureInput(text: $password, submit: submit)

            Button(action: submit) {
                Text("Unlock")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

private struct KeyTeleportSecureInput: View {
    @Binding var text: String
    let submit: () -> Void

    @State private var isRevealed = false

    var body: some View {
        HStack(spacing: 12) {
            Group {
                if isRevealed {
                    TextField(
                        "Teleport Password",
                        text: $text,
                        prompt: keyTeleportInputPlaceholder("Teleport Password")
                    )
                } else {
                    SecureField(
                        "Teleport Password",
                        text: $text,
                        prompt: keyTeleportInputPlaceholder("Teleport Password")
                    )
                }
            }
            .foregroundStyle(.white)
            .tint(.btnGradientLight)
            .textInputAutocapitalization(.characters)
            .autocorrectionDisabled()
            .submitLabel(.go)
            .onSubmit {
                guard !text.isEmpty else { return }

                submit()
            }

            Button {
                isRevealed.toggle()
            } label: {
                Image(systemName: isRevealed ? "eye.slash" : "eye")
                    .frame(width: 28, height: 28)
            }
            .buttonStyle(.plain)
            .foregroundStyle(.coveLightGray.opacity(0.82))
            .accessibilityLabel(isRevealed ? "Hide password" : "Show password")
        }
        .keyTeleportInputChrome()
    }
}

private struct KeyTeleportInputChrome: ViewModifier {
    func body(content: Content) -> some View {
        content
            .font(.body)
            .foregroundStyle(.white)
            .tint(.btnGradientLight)
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(Color.midnightBlue.opacity(0.62))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(Color.white.opacity(0.14), lineWidth: 1)
            )
    }
}

private extension View {
    func keyTeleportInputChrome() -> some View {
        modifier(KeyTeleportInputChrome())
    }
}

private func keyTeleportInputPlaceholder(_ title: LocalizedStringKey) -> Text {
    Text(title)
        .foregroundStyle(.coveLightGray.opacity(0.58))
}

private struct KeyTeleportMnemonicReviewView: View {
    let review: KeyTeleportMnemonicReview
    let words: [String]?
    let importWords: () -> Void
    let finish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Recovery words received", systemImage: "key.horizontal.fill")
                .font(.headline)

            Text("Cove found a \(review.wordCount)-word wallet. Review it below or import it directly.")
                .font(.subheadline)
                .foregroundStyle(.coveLightGray.opacity(0.74))

            if let words {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 120), spacing: 8)], spacing: 8) {
                    ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                        HStack {
                            Text("\(index + 1)")
                                .foregroundStyle(.coveLightGray.opacity(0.6))
                            Text(word)
                            Spacer()
                        }
                        .font(.system(.subheadline, design: .monospaced))
                        .padding(10)
                        .background(Color.midnightBlue.opacity(0.48))
                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
            } else {
                ProgressView()
                    .tint(.white)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
                    .accessibilityLabel("Loading recovery words")
            }

            Button("Import Wallet", action: importWords)
                .buttonStyle(OnboardingPrimaryButtonStyle())
                .disabled(words == nil)

            Button("Finish Without Importing", action: finish)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

private extension View {
    func protectedFromScreenCapture() -> some View {
        ScreenCaptureProtectedView {
            self
        }
    }
}

private struct ScreenCaptureProtectedView<Content: View>: UIViewControllerRepresentable {
    @ViewBuilder let content: Content

    func makeUIViewController(context _: Context) -> ScreenCaptureProtectedHostingController<Content> {
        ScreenCaptureProtectedHostingController(rootView: content)
    }

    func updateUIViewController(
        _ uiViewController: ScreenCaptureProtectedHostingController<Content>,
        context _: Context
    ) {
        uiViewController.rootView = content
    }

    func sizeThatFits(
        _ proposal: ProposedViewSize,
        uiViewController: ScreenCaptureProtectedHostingController<Content>,
        context _: Context
    ) -> CGSize? {
        uiViewController.sizeThatFits(proposal)
    }
}

private final class ScreenCaptureProtectedHostingController<Content: View>: UIViewController {
    private let secureTextField = UITextField()
    private let hostingController: UIHostingController<Content>

    var rootView: Content {
        get { hostingController.rootView }
        set { hostingController.rootView = newValue }
    }

    init(rootView: Content) {
        hostingController = UIHostingController(rootView: rootView)

        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func viewDidLoad() {
        super.viewDidLoad()

        view.backgroundColor = .clear
        configureSecureContainer()
    }

    private func configureSecureContainer() {
        secureTextField.isSecureTextEntry = true
        secureTextField.backgroundColor = .clear
        secureTextField.borderStyle = .none
        secureTextField.tintColor = .clear
        secureTextField.translatesAutoresizingMaskIntoConstraints = false

        view.addSubview(secureTextField)

        NSLayoutConstraint.activate([
            secureTextField.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            secureTextField.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            secureTextField.topAnchor.constraint(equalTo: view.topAnchor),
            secureTextField.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        secureTextField.layoutIfNeeded()

        let secureContainer = secureTextField.secureContentContainer ?? secureTextField
        addChild(hostingController)
        hostingController.view.backgroundColor = .clear
        hostingController.view.translatesAutoresizingMaskIntoConstraints = false
        secureContainer.addSubview(hostingController.view)

        NSLayoutConstraint.activate([
            hostingController.view.leadingAnchor.constraint(equalTo: secureContainer.leadingAnchor),
            hostingController.view.trailingAnchor.constraint(equalTo: secureContainer.trailingAnchor),
            hostingController.view.topAnchor.constraint(equalTo: secureContainer.topAnchor),
            hostingController.view.bottomAnchor.constraint(equalTo: secureContainer.bottomAnchor),
        ])

        hostingController.didMove(toParent: self)
    }

    func sizeThatFits(_ proposal: ProposedViewSize) -> CGSize {
        let fallbackWidth = view.bounds.width > 0 ? view.bounds.width : UIScreen.main.bounds.width
        let width = proposal.width ?? fallbackWidth
        let height = proposal.height ?? 10000

        return hostingController.sizeThatFits(in: CGSize(width: width, height: height))
    }
}

private extension UITextField {
    var secureContentContainer: UIView? {
        subviews.first { view in
            String(describing: type(of: view)).contains("Canvas")
        }
    }
}

private struct KeyTeleportXprvReviewView: View {
    let review: KeyTeleportXprvReview
    @Binding var xprv: String?
    let reveal: () -> Void
    let hide: () -> Void
    let importWallet: () -> Void
    let finish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("Extended private key received", systemImage: "key.horizontal.fill")
                .font(.headline)

            Text("Import this key as a hot wallet, or reveal it only when you are ready to handle the private key.")
                .font(.subheadline)
                .foregroundStyle(.coveLightGray.opacity(0.74))

            if review.revealed, let xprv {
                Text(xprv)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .padding()
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.midnightBlue.opacity(0.48))
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                HStack {
                    Button { copySensitiveText(xprv) } label: {
                        Label("Copy", systemImage: "doc.on.doc")
                    }
                    .buttonStyle(.bordered)
                    .tint(.white)

                    Button("Hide", action: hide)
                        .buttonStyle(.bordered)
                        .tint(.white)
                }
            } else {
                Text("Reveal only if you are ready to handle this private key.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)

                Button(action: reveal) {
                    Text("Reveal XPRV")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(OnboardingSecondaryButtonStyle())
            }

            Button("Import Wallet", action: importWallet)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Finish Without Importing", action: finish)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

private struct KeyTeleportMessageReviewView: View {
    let review: KeyTeleportMessageReview
    let finish: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Label(review.items.count == 1 ? "Message received" : "Messages received", systemImage: "note.text")
                .font(.headline)

            Text("This transfer contains text, not a wallet. Cove has displayed it exactly as received.")
                .font(.subheadline)
                .foregroundStyle(.coveLightGray.opacity(0.74))

            ForEach(Array(review.items.enumerated()), id: \.offset) { _, item in
                KeyTeleportMessageItemView(item: item)
            }

            Button("Done", action: finish)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

private struct KeyTeleportMessageItemView: View {
    let item: KeyTeleportMessageItem

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            switch item {
            case let .note(title, text, group):
                messageHeader(title: title, group: group, systemImage: "note.text")
                messageField(label: "Message", value: text)
            case let .password(title, username, password, site, notes, group):
                messageHeader(title: title, group: group, systemImage: "lock.fill")
                messageField(label: "Username", value: username)
                messageField(label: "Password", value: password)
                messageField(label: "Website", value: site)
                messageField(label: "Notes", value: notes)
            }
        }
        .padding(16)
        .background(Color.midnightBlue.opacity(0.48))
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private func messageHeader(title: String, group: String, systemImage: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Label(title, systemImage: systemImage)
                .font(.headline)

            Spacer()

            if !group.isEmpty {
                Text(group)
                    .font(.caption)
                    .foregroundStyle(.coveLightGray.opacity(0.7))
            }
        }
    }

    @ViewBuilder
    private func messageField(label: String, value: String) -> some View {
        if !value.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(label.uppercased())
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.coveLightGray.opacity(0.58))

                Text(value)
                    .font(.body)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

private struct KeyTeleportImportedWalletView: View {
    let wallet: WalletMetadata
    let finish: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            OnboardingStatusHero(
                systemImage: "checkmark",
                tint: .green,
                fillColor: .green.opacity(0.16),
                iconSize: 22,
                innerBadgeSize: 58
            )

            VStack(spacing: 8) {
                Text("Wallet imported")
                    .font(OnboardingRecoveryTypography.compactTitle)

                Text("\(wallet.name) is ready to use in Cove.")
                    .font(.subheadline)
                    .foregroundStyle(.coveLightGray.opacity(0.74))
                    .multilineTextAlignment(.center)
            }

            Button("Done", action: finish)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
        .frame(maxWidth: .infinity)
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
                .tint(.white)
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

            TextField(
                "Receiver Code",
                text: $code,
                prompt: keyTeleportInputPlaceholder("Receiver Code")
            )
            .keyboardType(.numberPad)
            .keyTeleportInputChrome()

            Button(action: submit) {
                Text("Continue")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())
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
            .buttonStyle(OnboardingPrimaryButtonStyle())
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
                .tint(.white)

                Button(action: share) {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                .buttonStyle(.bordered)
                .tint(.white)
            }

            Button("Done", action: finish)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

private struct KeyTeleportCardModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 22, style: .continuous)
                    .fill(Color.duskBlue.opacity(0.58))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 22, style: .continuous)
                    .stroke(Color.coveLightGray.opacity(0.12), lineWidth: 1)
            )
    }
}

private extension View {
    func keyTeleportCard() -> some View {
        modifier(KeyTeleportCardModifier())
    }
}

private func copySensitiveText(_ text: String) {
    UIPasteboard.general.setItems(
        [[UTType.utf8PlainText.identifier: text]],
        options: [
            .localOnly: true,
            .expirationDate: Date().addingTimeInterval(120),
        ]
    )
}
