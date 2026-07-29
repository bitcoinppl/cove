import SwiftUI

private enum TorSettingsMode: String, CaseIterable, Identifiable {
    case builtIn = "Built-in"
    case external = "External"

    var id: Self { self }
}

struct TorSettingsView: View {
    @State private var tor = TorManager.shared
    @State private var externalHost = "127.0.0.1"
    @State private var externalPort = "9050"
    @State private var disableWarning: TorDisableWarning?
    @State private var errorMessage: String?
    @State private var isUpdatingConfig = false
    @State private var isDispatchingTest = false

    private var isBuiltIn: Bool {
        tor.config == .builtIn
    }

    private var isExternal: Bool {
        if case .external = tor.config {
            return true
        }

        return false
    }

    private var selectedMode: Binding<TorSettingsMode> {
        Binding(
            get: { isExternal ? .external : .builtIn },
            set: { mode in
                Task { await setMode(mode) }
            }
        )
    }

    private var useTor: Binding<Bool> {
        Binding(
            get: { tor.isEnabled },
            set: { enabled in
                Task { await setTorEnabled(enabled) }
            }
        )
    }

    var body: some View {
        TorErrorAlertHost(errorMessage: $errorMessage) {
            TorDisableWarningAlertHost(
                disableWarning: $disableWarning,
                message: disableWarningMessage,
                confirmDisable: disableTorConfirmed
            ) {
                TorSettingsForm(
                    isEnabled: useTor,
                    isUpdatingConfig: isUpdatingConfig,
                    autoStartSuppressed: tor.autoStartSuppressed,
                    selectedMode: selectedMode,
                    isExternal: isExternal,
                    externalHost: $externalHost,
                    externalPort: $externalPort,
                    status: tor.status,
                    isBuiltIn: isBuiltIn,
                    connectionTestStates: tor.connectionTestStates,
                    connectionTestError: tor.connectionTestError,
                    isConnectionTestRunning: isDispatchingTest || tor.isConnectionTestRunning,
                    startBuiltInTor: enableTor,
                    saveExternalProxy: saveExternalProxy,
                    runConnectionTest: runConnectionTest
                )
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Tor")
        .onChange(of: tor.config, initial: true) { _, config in
            syncExternalFields(with: config)
        }
    }

    @MainActor
    private func setTorEnabled(_ enabled: Bool) async {
        guard !enabled else { return await enableTor() }

        isUpdatingConfig = true
        defer { isUpdatingConfig = false }

        do {
            if let warning = try await tor.disable() {
                disableWarning = warning
            }
        } catch {
            errorMessage = torErrorMessage(error)
        }
    }

    @MainActor
    private func enableTor() async {
        isUpdatingConfig = true
        defer { isUpdatingConfig = false }

        do {
            try await tor.enable()
        } catch {
            errorMessage = torErrorMessage(error)
        }
    }

    @MainActor
    private func setMode(_ mode: TorSettingsMode) async {
        switch mode {
        case .builtIn:
            await applyConfig(.builtIn)

        case .external:
            guard let config = externalConfig() else { return }
            await applyConfig(config)
        }
    }

    @MainActor
    private func saveExternalProxy() async {
        guard let config = externalConfig() else { return }
        await applyConfig(config)
    }

    @MainActor
    private func applyConfig(_ config: TorConfig) async {
        isUpdatingConfig = true
        defer { isUpdatingConfig = false }

        do {
            if let warning = try await tor.setConfig(config) {
                disableWarning = warning
            }
        } catch {
            errorMessage = torErrorMessage(error)
        }
    }

    @MainActor
    private func disableTorConfirmed() async {
        isUpdatingConfig = true
        defer { isUpdatingConfig = false }

        do {
            try await tor.disableConfirmed()
        } catch {
            errorMessage = torErrorMessage(error)
        }
    }

    @MainActor
    private func runConnectionTest() async {
        isDispatchingTest = true
        defer { isDispatchingTest = false }

        do {
            try await tor.runConnectionTest()
        } catch {
            errorMessage = torErrorMessage(error)
        }
    }

    @MainActor
    private func externalConfig() -> TorConfig? {
        let host = externalHost.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !host.isEmpty else {
            errorMessage = "Enter a SOCKS5 proxy host."
            return nil
        }

        guard let port = UInt16(externalPort), port > 0 else {
            errorMessage = "Enter a numeric port between 1 and 65535."
            return nil
        }

        return .external(host: host, port: port)
    }

    private func syncExternalFields(with config: TorConfig) {
        guard case let .external(host, port) = config else { return }

        externalHost = host
        externalPort = String(port)
    }

    private var disableWarningMessage: String {
        guard case let .onionNodesSelected(networks) = disableWarning else { return "" }

        let names = networks.map { $0.displayName() }.formatted(.list(type: .and))
        if networks.count == 1 {
            return "Your selected \(names) node uses an onion address. Cove will switch it to a clearnet server before turning Tor off."
        }

        return "Your selected nodes on \(names) use onion addresses. Cove will switch them to clearnet servers before turning Tor off."
    }

    private func torErrorMessage(_ error: Error) -> String {
        if let error = error as? TorError {
            return error.description
        }

        return error.localizedDescription
    }
}

private struct TorDisableWarningAlertHost<Content: View>: View {
    @Binding var disableWarning: TorDisableWarning?
    let message: String
    let confirmDisable: () async -> Void
    @ViewBuilder let content: Content

    private var isPresented: Binding<Bool> {
        Binding(
            get: { disableWarning != nil },
            set: { if !$0 { disableWarning = nil } }
        )
    }

    var body: some View {
        content
            .alert("Disable Tor?", isPresented: isPresented) {
                Button("Disable and Switch Node", role: .destructive) {
                    disableWarning = nil
                    Task { await confirmDisable() }
                }
                Button("Cancel", role: .cancel) {
                    disableWarning = nil
                }
            } message: {
                Text(message)
            }
    }
}

private struct TorErrorAlertHost<Content: View>: View {
    @Binding var errorMessage: String?
    @ViewBuilder let content: Content

    private var isPresented: Binding<Bool> {
        Binding(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )
    }

    var body: some View {
        content
            .alert("Unable to Update Tor", isPresented: isPresented) {
                Button("OK", role: .cancel) {
                    errorMessage = nil
                }
            } message: {
                Text(errorMessage ?? "")
            }
    }
}

private struct TorSettingsForm: View {
    @Binding var isEnabled: Bool
    let isUpdatingConfig: Bool
    let autoStartSuppressed: Bool
    @Binding var selectedMode: TorSettingsMode
    let isExternal: Bool
    @Binding var externalHost: String
    @Binding var externalPort: String
    let status: TorStatus
    let isBuiltIn: Bool
    let connectionTestStates: [TorTestStep: TorTestState]
    let connectionTestError: String?
    let isConnectionTestRunning: Bool
    let startBuiltInTor: () async -> Void
    let saveExternalProxy: () async -> Void
    let runConnectionTest: () async -> Void

    var body: some View {
        Form {
            TorEnableSection(isEnabled: $isEnabled, isUpdatingConfig: isUpdatingConfig)

            if isEnabled {
                TorAutoStartSuppressedSection(
                    isVisible: autoStartSuppressed,
                    isUpdatingConfig: isUpdatingConfig,
                    startBuiltInTor: startBuiltInTor
                )
                TorModeSection(
                    selectedMode: $selectedMode,
                    isUpdatingConfig: isUpdatingConfig
                )

                if isExternal {
                    TorExternalProxySection(
                        host: $externalHost,
                        port: $externalPort,
                        isUpdatingConfig: isUpdatingConfig,
                        save: saveExternalProxy
                    )
                }

                TorConnectionStatusSection(
                    status: status,
                    isBuiltIn: isBuiltIn,
                    isExternal: isExternal
                )
                TorConnectionTestSection(
                    states: connectionTestStates,
                    errorMessage: connectionTestError,
                    isRunning: isConnectionTestRunning,
                    run: runConnectionTest
                )
            }
        }
    }
}

private struct TorEnableSection: View {
    @Binding var isEnabled: Bool
    let isUpdatingConfig: Bool

    var body: some View {
        Section {
            Toggle(isOn: $isEnabled) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Use Tor")
                    Text("Route network traffic through Tor for enhanced privacy")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .disabled(isUpdatingConfig)
        }
    }
}

private struct TorAutoStartSuppressedSection: View {
    let isVisible: Bool
    let isUpdatingConfig: Bool
    let startBuiltInTor: () async -> Void

    var body: some View {
        if isVisible {
            Section {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Built-in Tor auto-start is paused after repeated failures")
                    Text(
                        "Cove stopped starting Tor automatically because it failed to launch several times in a row. Network requests stay blocked until Tor starts."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }

                Button("Start Built-in Tor") {
                    Task { await startBuiltInTor() }
                }
                .disabled(isUpdatingConfig)
            }
        }
    }
}

private struct TorModeSection: View {
    @Binding var selectedMode: TorSettingsMode
    let isUpdatingConfig: Bool

    var body: some View {
        Section("Connection Mode") {
            Picker("Mode", selection: $selectedMode) {
                ForEach(TorSettingsMode.allCases) { mode in
                    Text(mode.rawValue).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .disabled(isUpdatingConfig)
        }
    }
}

private struct TorExternalProxySection: View {
    @Binding var host: String
    @Binding var port: String
    let isUpdatingConfig: Bool
    let save: () async -> Void

    var body: some View {
        Section {
            TextField("Host", text: $host)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            TextField("Port", text: $port)
                .keyboardType(.numberPad)
            Button("Save Proxy") {
                Task { await save() }
            }
            .disabled(isUpdatingConfig || host.isEmpty || port.isEmpty)
        } header: {
            Text("External SOCKS5 Proxy")
        } footer: {
            Text("Enter the host and port of a SOCKS5 proxy managed outside Cove.")
        }
    }
}

private struct TorConnectionStatusSection: View {
    let status: TorStatus
    let isBuiltIn: Bool
    let isExternal: Bool

    var body: some View {
        Section("Connection Status") {
            TorConnectionStatusRow(status: status, isExternal: isExternal)

            if isBuiltIn, case let .bootstrapping(percent, message) = status {
                TorBootstrapProgress(percent: percent, message: message)
            }
        }
    }
}

private struct TorConnectionStatusRow: View {
    let status: TorStatus
    let isExternal: Bool

    var body: some View {
        HStack(spacing: 12) {
            Circle()
                .fill(statusColor)
                .frame(width: 10, height: 10)

            VStack(alignment: .leading, spacing: 2) {
                Text(statusTitle)

                if let statusDetail {
                    Text(statusDetail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private var statusTitle: String {
        switch status {
        case .off:
            "Off"
        case let .bootstrapping(percent, _):
            "Connecting · \(percent)%"
        case .ready:
            "Ready"
        case .stopped where isExternal:
            "External proxy configured"
        case .stopped:
            "Stopped"
        case .failed:
            "Unavailable"
        }
    }

    private var statusDetail: String? {
        switch status {
        case let .bootstrapping(_, message), let .failed(message):
            message
        case .stopped where isExternal:
            "Run the connection test to verify the proxy."
        case .off, .ready, .stopped:
            nil
        }
    }

    private var statusColor: Color {
        switch status {
        case .off, .stopped:
            .secondary
        case .bootstrapping:
            .statusWarning
        case .ready:
            .statusSuccess
        case .failed:
            .statusError
        }
    }
}

private struct TorBootstrapProgress: View {
    let percent: UInt8
    let message: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Bootstrap Progress")
                Spacer()
                Text("\(percent)%")
                    .foregroundStyle(.secondary)
            }
            ProgressView(value: Double(percent), total: 100)
            Text(message)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }
}

private struct TorConnectionTestSection: View {
    let states: [TorTestStep: TorTestState]
    let errorMessage: String?
    let isRunning: Bool
    let run: () async -> Void

    var body: some View {
        Section {
            TorTestStepRow(step: .proxyReachable, state: states[.proxyReachable])
            TorTestStepRow(step: .nodeReachableViaTor, state: states[.nodeReachableViaTor])
            Button("Test Connection") {
                Task { await run() }
            }
            .disabled(isRunning)
        } header: {
            Text("Connection Test")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                Text("Tests the SOCKS5 proxy and your selected node through the configured Tor route.")

                if let errorMessage {
                    Text("The last test could not finish: \(errorMessage)")
                        .foregroundStyle(Color.statusError)
                }
            }
        }
    }
}

private struct TorTestStepRow: View {
    let step: TorTestStep
    let state: TorTestState?

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .foregroundStyle(color)
                .frame(width: 20)

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            if state == .running {
                ProgressView()
            }
        }
    }

    private var title: String {
        switch step {
        case .proxyReachable:
            "SOCKS5 Proxy"
        case .nodeReachableViaTor:
            "Selected Node"
        }
    }

    private var detail: String {
        switch state {
        case nil:
            "Not tested"
        case .running:
            "Testing…"
        case .passed:
            "Connected"
        case let .failed(message):
            message
        }
    }

    private var symbol: String {
        switch state {
        case nil:
            "circle"
        case .running:
            "circle.dotted"
        case .passed:
            "checkmark.circle.fill"
        case .failed:
            "xmark.circle.fill"
        }
    }

    private var color: Color {
        switch state {
        case nil:
            .secondary
        case .running:
            .statusWarning
        case .passed:
            .statusSuccess
        case .failed:
            .statusError
        }
    }
}

#Preview {
    NavigationStack {
        TorSettingsView()
    }
}
