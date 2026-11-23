import SwiftUI

/// The option the Mode picker shows, which runs ahead of the saved mode while the user fills in
/// a proxy they haven't saved yet
enum TorModeChoice: CaseIterable, Hashable {
    case off
    case builtIn
    case proxy

    init(_ mode: TorMode) {
        self = switch mode {
        case .off: .off
        case .builtIn: .builtIn
        case .proxy: .proxy
        }
    }

    var title: String {
        switch self {
        case .off: "Off"
        case .builtIn: "Built-in Tor"
        case .proxy: "Tor proxy"
        }
    }

    var footer: String {
        switch self {
        case .off:
            "Cove connects to your node and to price and fee servers directly."
        case .builtIn:
            "Cove runs Tor itself. The first start takes a few seconds while Tor joins the network."
        case .proxy:
            "Use a Tor SOCKS5 proxy you run, such as Orbot."
        }
    }

    var accessibilityIdentifier: String {
        switch self {
        case .off: "tor.mode.off"
        case .builtIn: "tor.mode.builtIn"
        case .proxy: "tor.mode.proxy"
        }
    }
}

extension TorMode {
    /// The short name the Settings row shows next to "Tor"
    var settingsSummary: String {
        switch self {
        case .off: "Off"
        case .builtIn: "Built-in"
        case .proxy: "Proxy"
        }
    }
}

/// Where the Check Connection button stands, reset whenever the saved mode changes since a result
/// only describes the route it was checked on
enum TorConnectionCheckState: Equatable {
    case idle
    case checking
    case finished(Result<TorConnectionCheck, TorManagerError>)

    var isChecking: Bool {
        self == .checking
    }
}

struct TorSettingsScreen: View {
    @State private var tor = TorManager.shared

    @State private var choice: TorModeChoice?
    @State private var proxyInput: String
    @State private var proxyError: String?
    @State private var check = TorConnectionCheckState.idle
    @State private var checkTask: Task<Void, Never>?
    @FocusState private var isProxyFocused: Bool

    init() {
        let tor = TorManager.shared
        _choice = State(initialValue: tor.mode.map(TorModeChoice.init))
        _proxyInput = State(initialValue: tor.mode?.proxyAddress ?? tor.defaultProxyAddress())
    }

    /// Checking needs a Tor route, which only a saved mode other than Off has
    private var isCheckAvailable: Bool {
        tor.mode.map { $0 != .off } ?? false
    }

    var body: some View {
        Form {
            TorModeSection(choice: choice, onSelect: select)
            TorProxySection(
                isVisible: choice == .proxy,
                input: $proxyInput,
                error: proxyError,
                isFocused: $isProxyFocused,
                onInputChanged: clearProxyError,
                onUse: useProxy
            )
            TorStatusSection(
                status: tor.status,
                isCheckAvailable: isCheckAvailable,
                check: check,
                onRestart: restart,
                onCheck: checkConnection
            )
            TorAboutSection()
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Tor")
        .onChange(of: tor.mode, modeChanged)
        .onChange(of: tor.modeRejection, modeRejectionChanged)
        .onDisappear(perform: disappear)
        .modifier(TorModeRejectionAlert(rejection: tor.modeRejection, onDismiss: tor.dismissModeRejection))
    }

    private func select(_ next: TorModeChoice) {
        guard next != choice else { return }

        choice = next
        proxyError = nil

        // picking the saved mode again would restart Tor for nothing
        if let mode = tor.mode, TorModeChoice(mode) == next { return }

        switch next {
        case .off:
            tor.dispatch(action: .selectMode(.off))
        case .builtIn:
            tor.dispatch(action: .selectMode(.builtIn))
        // a proxy saves only once the user confirms its address with Use Proxy
        case .proxy:
            break
        }
    }

    private func useProxy() {
        switch tor.validateProxyAddress(proxyInput) {
        case let .success(address):
            proxyInput = address
            proxyError = nil
            isProxyFocused = false
            tor.dispatch(action: .selectMode(.proxy(address: address)))
        case let .failure(error):
            proxyError = error.description
        }
    }

    private func clearProxyError() {
        proxyError = nil
    }

    private func restart() {
        tor.dispatch(action: .restart)
    }

    private func checkConnection() {
        checkTask?.cancel()
        check = .checking

        checkTask = Task {
            let result = await tor.checkConnection()

            // the mode changed while the check ran, so its answer is about a route that's gone
            guard !Task.isCancelled else { return }

            check = .finished(result)
            checkTask = nil
        }
    }

    private func resetCheck() {
        checkTask?.cancel()
        checkTask = nil
        check = .idle
    }

    private func modeChanged(_: TorMode?, _ mode: TorMode?) {
        resetCheck()

        // an open proxy form holds the user's unsaved address, so a mode saved for an earlier
        // pick leaves it open
        if choice == .proxy { return }

        choice = mode.map(TorModeChoice.init)
    }

    /// A refused change leaves the saved mode as it was, so the picker goes back to it
    private func modeRejectionChanged(_: TaggedItem<TorManagerError>?, _ rejection: TaggedItem<TorManagerError>?) {
        guard rejection != nil else { return }

        choice = tor.mode.map(TorModeChoice.init)
    }

    private func disappear() {
        resetCheck()

        // a refusal that arrives after the user left would otherwise pop up on the next visit
        tor.dismissModeRejection()
    }
}

private struct TorModeRejectionAlert: ViewModifier {
    let rejection: TaggedItem<TorManagerError>?
    let onDismiss: () -> Void

    private var isPresented: Binding<Bool> {
        Binding(
            get: { rejection != nil },
            set: { if !$0 { onDismiss() } }
        )
    }

    func body(content: Content) -> some View {
        content
            .alert("Couldn't Change Tor", isPresented: isPresented, presenting: rejection) { _ in
                Button("OK", role: .cancel, action: onDismiss)
            } message: { rejection in
                Text(rejection.item.description)
            }
    }
}

private struct TorModeSection: View {
    let choice: TorModeChoice?
    let onSelect: (TorModeChoice) -> Void

    var body: some View {
        Section {
            ForEach(TorModeChoice.allCases, id: \.self) { option in
                TorModeRow(option: option, isSelected: option == choice, onSelect: onSelect)
            }
        } header: {
            Text("Mode")
        } footer: {
            if let choice {
                Text(choice.footer)
            }
        }
    }
}

private struct TorModeRow: View {
    let option: TorModeChoice
    let isSelected: Bool
    let onSelect: (TorModeChoice) -> Void

    var body: some View {
        Button(action: select) {
            HStack {
                Text(option.title)
                    .foregroundStyle(.primary)

                Spacer()

                if isSelected {
                    Image(systemName: "checkmark")
                        .foregroundStyle(.blue)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
            }
            .contentShape(Rectangle())
        }
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityIdentifier(option.accessibilityIdentifier)
    }

    private func select() {
        onSelect(option)
    }
}

private struct TorProxySection: View {
    let isVisible: Bool
    @Binding var input: String
    let error: String?
    let isFocused: FocusState<Bool>.Binding
    let onInputChanged: () -> Void
    let onUse: () -> Void

    var body: some View {
        if isVisible {
            Section("Proxy") {
                TorProxyAddressField(
                    input: $input,
                    error: error,
                    isFocused: isFocused,
                    onInputChanged: onInputChanged,
                    onSubmit: onUse
                )

                Button("Use Proxy", action: onUse)
                    .accessibilityIdentifier("tor.proxy.use")
            }
        }
    }
}

private struct TorProxyAddressField: View {
    @Binding var input: String
    let error: String?
    let isFocused: FocusState<Bool>.Binding
    let onInputChanged: () -> Void
    let onSubmit: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            LabeledContent("Proxy address") {
                TextField("Proxy address", text: $input)
                    // host:port is digits, dots and a colon, which this keyboard shows first
                    .keyboardType(.numbersAndPunctuation)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .multilineTextAlignment(.trailing)
                    .lineLimit(1)
                    .submitLabel(.done)
                    .focused(isFocused)
                    .onChange(of: input, inputChanged)
                    .onSubmit(onSubmit)
                    .accessibilityIdentifier("tor.proxy.address")
            }

            if let error {
                Text(error)
                    .font(.footnote)
                    .foregroundStyle(Color.statusError)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("tor.proxy.error")
            }
        }
    }

    private func inputChanged(_: String, _: String) {
        onInputChanged()
    }
}

private struct TorStatusSection: View {
    let status: TorStatus
    let isCheckAvailable: Bool
    let check: TorConnectionCheckState
    let onRestart: () -> Void
    let onCheck: () -> Void

    var body: some View {
        Section {
            TorStatusRow(status: status, onRestart: onRestart)
            TorConnectionCheckRows(isVisible: isCheckAvailable, check: check, onCheck: onCheck)
        }
    }
}

/// What the Status row says for each `TorStatus`
private struct TorStatusPresentation {
    let title: String
    let message: String?
    let isConnecting: Bool
    let canRetry: Bool

    init(_ status: TorStatus) {
        switch status {
        case .off:
            self.init(title: "Off")
        case let .starting(percent):
            self.init(title: "Connecting… \(percent)%", isConnecting: true)
        case .ready:
            self.init(title: "Connected")
        case let .failed(message):
            self.init(title: "Not connected", message: message, canRetry: true)
        case .usingProxy:
            self.init(title: "Using proxy")
        }
    }

    private init(title: String, message: String? = nil, isConnecting: Bool = false, canRetry: Bool = false) {
        self.title = title
        self.message = message
        self.isConnecting = isConnecting
        self.canRetry = canRetry
    }
}

private struct TorStatusRow: View {
    let onRestart: () -> Void
    private let presentation: TorStatusPresentation

    init(status: TorStatus, onRestart: @escaping () -> Void) {
        self.onRestart = onRestart
        presentation = TorStatusPresentation(status)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Status")

                Spacer()

                if presentation.isConnecting {
                    ProgressView()
                        .controlSize(.small)
                }

                Text(presentation.title)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("tor.status")
            }

            if let message = presentation.message {
                Text(message)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("tor.status.message")
            }
        }

        if presentation.canRetry {
            Button("Try Again", action: onRestart)
                .accessibilityIdentifier("tor.restart")
        }
    }
}

private struct TorConnectionCheckRows: View {
    let isVisible: Bool
    let check: TorConnectionCheckState
    let onCheck: () -> Void

    var body: some View {
        if isVisible {
            Button(action: onCheck) {
                HStack {
                    Text("Check Connection")

                    Spacer()

                    if check.isChecking {
                        ProgressView()
                            .controlSize(.small)
                    }
                }
                .contentShape(Rectangle())
            }
            .disabled(check.isChecking)
            .accessibilityIdentifier("tor.checkConnection")

            TorConnectionCheckResult(check: check)
        }
    }
}

private struct TorConnectionCheckResult: View {
    private let text: String?
    private let color: Color

    init(check: TorConnectionCheckState) {
        switch check {
        case .idle, .checking:
            text = nil
            color = .secondary
        case let .finished(.success(result)) where result.isTor:
            text = "Traffic is going through Tor"
            color = .statusSuccess
        case .finished(.success):
            text = "This proxy isn't sending traffic through Tor"
            color = .statusWarning
        case let .finished(.failure(error)):
            text = error.description
            color = .statusError
        }
    }

    var body: some View {
        if let text {
            Text(text)
                .font(.footnote)
                .foregroundStyle(color)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("tor.checkConnection.result")
        }
    }
}

private struct TorAboutSection: View {
    var body: some View {
        Section("About") {
            Text(
                "While Tor is on, connections to your node, fee and price updates, Payjoin, and diagnostics reports go through Tor. If Tor isn't connected, Cove waits instead of connecting directly. iCloud backups and links you open in your browser use your regular connection."
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
    }
}

#Preview {
    NavigationStack {
        TorSettingsScreen()
    }
    .environment(AppManager.shared)
}
