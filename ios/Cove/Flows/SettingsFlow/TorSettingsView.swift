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
        Form {
            Section {
                Toggle(isOn: useTor) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Use Tor")

                        Text("Route network traffic through Tor for enhanced privacy")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .disabled(isUpdatingConfig)
            }

            if tor.isEnabled {
                modeSection

                if isExternal {
                    externalProxySection
                }

                statusSection
                connectionTestSection
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Tor")
        .onChange(of: tor.config, initial: true) { _, config in
            syncExternalFields(with: config)
        }
        .alert("Disable Tor?", isPresented: Binding(
            get: { disableWarning != nil },
            set: { if !$0 { disableWarning = nil } }
        )) {
            Button("Disable and Switch Node", role: .destructive) {
                disableWarning = nil
                Task { await disableTorConfirmed() }
            }

            Button("Cancel", role: .cancel) {
                disableWarning = nil
            }
        } message: {
            Text(
                "Your selected node uses an onion address. Cove must switch to a clearnet node before Tor can be disabled."
            )
        }
        .alert("Unable to Update Tor", isPresented: Binding(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )) {
            Button("OK", role: .cancel) {
                errorMessage = nil
            }
        } message: {
            Text(errorMessage ?? "")
        }
    }

    private var modeSection: some View {
        Section("Connection Mode") {
            Picker("Mode", selection: selectedMode) {
                ForEach(TorSettingsMode.allCases) { mode in
                    Text(mode.rawValue).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .disabled(isUpdatingConfig)
        }
    }

    private var externalProxySection: some View {
        Section {
            TextField("Host", text: $externalHost)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()

            TextField("Port", text: $externalPort)
                .keyboardType(.numberPad)

            Button("Save Proxy") {
                Task { await saveExternalProxy() }
            }
            .disabled(isUpdatingConfig || externalHost.isEmpty || externalPort.isEmpty)
        } header: {
            Text("External SOCKS5 Proxy")
        } footer: {
            Text("Enter the host and port of a SOCKS5 proxy managed outside Cove.")
        }
    }

    private var statusSection: some View {
        Section("Connection Status") {
            HStack(spacing: 12) {
                Circle()
                    .fill(statusColor)
                    .frame(width: 10, height: 10)

                VStack(alignment: .leading, spacing: 2) {
                    Text(statusTitle)

                    if let detail = statusDetail {
                        Text(detail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            if isBuiltIn, case let .bootstrapping(percent, message) = tor.status {
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
    }

    private var connectionTestSection: some View {
        Section {
            testStepRow(.proxyReachable)
            testStepRow(.nodeReachableViaTor)

            Button("Test Connection") {
                Task { await runConnectionTest() }
            }
            .disabled(isDispatchingTest || tor.isConnectionTestRunning)
        } header: {
            Text("Connection Test")
        } footer: {
            Text("Tests the SOCKS5 proxy and your selected node through the configured Tor route.")
        }
    }

    private func testStepRow(_ step: TorTestStep) -> some View {
        let state = tor.connectionTestStates[step]

        return HStack(spacing: 12) {
            Image(systemName: testStepSymbol(state))
                .foregroundStyle(testStepColor(state))
                .frame(width: 20)

            VStack(alignment: .leading, spacing: 2) {
                Text(testStepTitle(step))

                Text(testStepDetail(state))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            if state == .running {
                ProgressView()
            }
        }
    }

    @MainActor
    private func setTorEnabled(_ enabled: Bool) async {
        isUpdatingConfig = true
        defer { isUpdatingConfig = false }

        do {
            if enabled {
                try await tor.enable()
            } else if let warning = try await tor.disable() {
                disableWarning = warning
            }
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

    private var statusTitle: String {
        switch tor.status {
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
        switch tor.status {
        case let .bootstrapping(_, message), let .failed(message):
            message
        case .stopped where isExternal:
            "Run the connection test to verify the proxy."
        case .off, .ready, .stopped:
            nil
        }
    }

    private var statusColor: Color {
        switch tor.status {
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

    private func testStepTitle(_ step: TorTestStep) -> String {
        switch step {
        case .proxyReachable:
            "SOCKS5 Proxy"
        case .nodeReachableViaTor:
            "Selected Node"
        }
    }

    private func testStepDetail(_ state: TorTestState?) -> String {
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

    private func testStepSymbol(_ state: TorTestState?) -> String {
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

    private func testStepColor(_ state: TorTestState?) -> Color {
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

    private func torErrorMessage(_ error: Error) -> String {
        if let error = error as? TorError {
            return error.description
        }

        return error.localizedDescription
    }
}

#Preview {
    NavigationStack {
        TorSettingsView()
    }
}
