import SwiftUI
import UIKit

private enum TorTestStepStatus: Equatable {
    case pending
    case running
    case passed
    case failed
}

private struct TorTestStep: Equatable, Identifiable {
    var id: String {
        key
    }

    let key: String
    var title: String
    var detail: String
    var status: TorTestStepStatus = .pending
}

private struct TorConnectionTestState: Equatable {
    var running = false
    var finished = false
    var steps: [TorTestStep] = []
    var logs: [String] = []
}

struct NetworkSettingsView: View {
    @Binding var selection: Network

    @Environment(AppManager.self) private var app

    private let db = Database()
    private let nodeSelector = NodeSelector()

    @State private var pendingNetwork: Network?
    @State private var torSettingsDiscovered = false
    @State private var uiState = TorUiState()
    @State private var showModeDialog = false
    @State private var showFullLogSheet = false
    @State private var showTorTestSheet = false
    @State private var showDisableTorOnionAlert = false
    @State private var rustTorLogCount = 0
    @State private var builtInWarmupRequested = false
    @State private var torTestState = TorConnectionTestState()
    @State private var autoPendingTestKey: String?
    @State private var noticeMessage: String?

    private var globalConfig: GlobalConfigTable {
        db.globalConfig()
    }

    private var globalFlag: GlobalFlagTable {
        db.globalFlag()
    }

    var body: some View {
        Form {
            networkSection

            if torSettingsDiscovered {
                torMainSection

                if uiState.enabled {
                    switch uiState.mode {
                    case .builtIn:
                        builtInSection
                    case .external:
                        externalSection
                    case .orbot:
                        orbotSection
                    }
                }
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Network")
        .onAppear(perform: loadPersistedTorState)
        .task {
            await refreshInitialTorState()
        }
        .task(id: "\(uiState.enabled)-\(uiState.mode.persistedValue)") {
            await pollBuiltInTorLogsIfNeeded()
        }
        .onChange(of: uiState.status) { _, _ in
            Task { await maybeRunPendingOnionValidation() }
        }
        .alert("Change Network?", isPresented: Binding(
            get: { pendingNetwork != nil },
            set: { if !$0 { pendingNetwork = nil } }
        )) {
            Button("Yes, Change Network") {
                if let network = pendingNetwork {
                    app.dispatch(action: .changeNetwork(network: network))
                    app.rust.selectLatestOrNewWallet()
                    selection = network
                    app.popRoute()
                }
                pendingNetwork = nil
            }
            Button("Cancel", role: .cancel) {
                pendingNetwork = nil
            }
        } message: {
            if let network = pendingNetwork {
                Text("Switching to \(network.displayName) will take you to a wallet on that network.")
            }
        }
        .alert("Disable Tor?", isPresented: $showDisableTorOnionAlert) {
            Button("Disable and switch node") {
                Task { await disableTorWithClearnetFallback() }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Your active or pending node uses an onion address. Disable Tor only if Cove first switches to a clearnet node.")
        }
        .alert("Network", isPresented: Binding(
            get: { noticeMessage != nil },
            set: { if !$0 { noticeMessage = nil } }
        )) {
            Button("OK") { noticeMessage = nil }
        } message: {
            Text(noticeMessage ?? "")
        }
        .confirmationDialog("Connection Mode", isPresented: $showModeDialog, titleVisibility: .visible) {
            ForEach(TorMode.uiCases, id: \.self) { mode in
                Button(mode.title) {
                    setTorMode(mode)
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .sheet(isPresented: $showFullLogSheet) {
            TorFullLogSheet(logLines: uiState.logLines)
        }
        .sheet(isPresented: $showTorTestSheet) {
            TorTestSheetContent(state: torTestState) {
                if !torTestState.running {
                    showTorTestSheet = false
                }
            }
            .interactiveDismissDisabled(torTestState.running)
            .presentationDetents([.height(560), .large])
        }
    }

    private var networkSection: some View {
        Section {
            ForEach(Network.allCases, id: \.self) { item in
                HStack {
                    if !item.symbol.isEmpty {
                        Image(systemName: item.symbol)
                    }

                    Text(item.displayName)
                        .font(.subheadline)

                    Spacer()

                    if selection == item {
                        Image(systemName: "checkmark")
                            .foregroundStyle(.blue)
                            .font(.footnote)
                            .fontWeight(.semibold)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture {
                    if item != selection {
                        pendingNetwork = item
                    }
                }
            }
        }
    }

    private var torMainSection: some View {
        Section("Tor Network") {
            Toggle(isOn: Binding(
                get: { uiState.enabled },
                set: handleUseTorToggle
            )) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Use Tor")
                    Text("Route traffic through the Tor network for enhanced privacy")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Menu {
                ForEach(TorMode.uiCases, id: \.self) { mode in
                    Button(mode.title) {
                        setTorMode(mode)
                    }
                }
            } label: {
                HStack {
                    Label("Connection Mode", systemImage: "network")
                    Spacer()
                    Text(uiState.mode.shortTitle)
                        .foregroundStyle(.blue)
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
            .disabled(!uiState.enabled)
            .opacity(uiState.enabled ? 1 : 0.5)

            HStack {
                Label("Tor Status", systemImage: "info.circle")
                Spacer()
                Text(statusLabel)
                    .foregroundStyle(.secondary)
                if uiState.enabled, uiState.status == .ready {
                    Image(systemName: "checkmark")
                        .foregroundStyle(.blue)
                }
            }
            .opacity(uiState.enabled ? 1 : 0.5)
        }
    }

    private var builtInSection: some View {
        Group {
            Section("Connection Status") {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text("Bootstrap Progress")
                        Spacer()
                        Text("\(uiState.progressPercent)%")
                            .foregroundStyle(.blue)
                    }

                    ProgressView(value: Double(uiState.progressPercent), total: 100)

                    Text("Status: \(uiState.currentStep)")
                        .font(.subheadline)

                    Button("Test connection") {
                        Task { await runProgressiveTorTest() }
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.blue)
                    .frame(maxWidth: .infinity)
                }
                .padding(.vertical, 4)
            }

            Section {
                Button {
                    appendTorLog("Opened Tor connection logs")
                    syncRustTorLogs()
                    showFullLogSheet = true
                } label: {
                    Label("Connection Logs", systemImage: "terminal")
                }
            } footer: {
                Text("View detailed Tor bootstrap logs")
            }
        }
    }

    private var externalSection: some View {
        Section("Custom Proxy Settings") {
            Text("Configure your custom Tor proxy or SOCKS5 bridge")
                .font(.caption)
                .foregroundStyle(.secondary)

            TextField("SOCKS Host", text: Binding(
                get: { uiState.externalHost },
                set: { value in
                    uiState.externalHost = value
                    uiState.externalValidationError = validateTorExternalConfig(
                        host: value,
                        port: uiState.externalPort
                    )
                }
            ))
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()

            TextField("SOCKS Port", text: Binding(
                get: { uiState.externalPort },
                set: { value in
                    uiState.externalPort = value
                    uiState.externalValidationError = validateTorExternalConfig(
                        host: uiState.externalHost,
                        port: value
                    )
                }
            ))
            .keyboardType(.numberPad)

            if let error = uiState.externalValidationError {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            HStack {
                Button("Save config") {
                    saveExternalConfig()
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.blue)
                .frame(maxWidth: .infinity)

                Button("Test connection") {
                    Task { await runProgressiveTorTest() }
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.blue)
                .frame(maxWidth: .infinity)
            }
        }
    }

    private var orbotSection: some View {
        Section {
            HStack {
                Label(orbotTitle, systemImage: "gearshape")
                Spacer()
                if uiState.orbotStatus == .checking {
                    ProgressView()
                }
            }

            Button {
                Task { await refreshOrbotStatus() }
            } label: {
                Label("Refresh Status", systemImage: "arrow.clockwise")
            }

            HStack {
                Button("Open Orbot") {
                    openOrbotBestEffort()
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.blue)
                .frame(maxWidth: .infinity)

                Button("Test connection") {
                    guard uiState.orbotStatus == .detected else {
                        noticeMessage = "Orbot SOCKS endpoint is not reachable at 127.0.0.1:9050."
                        return
                    }
                    Task { await runProgressiveTorTest() }
                }
                .buttonStyle(.borderless)
                .foregroundStyle(.blue)
                .frame(maxWidth: .infinity)
            }
        } header: {
            Text("Orbot Integration")
        } footer: {
            Text("On iOS, Orbot mode is validated by checking the local SOCKS endpoint at 127.0.0.1:9050.")
        }
    }

    private var statusLabel: String {
        switch uiState.status {
        case .disabled:
            "Disabled"
        case .bootstrapping:
            "Needs testing"
        case .ready:
            "Configured"
        case .error:
            "Action required"
        }
    }

    private var orbotTitle: String {
        switch uiState.orbotStatus {
        case .checking:
            "Checking Orbot..."
        case .detected:
            "Orbot SOCKS Endpoint Reachable"
        case .notDetected:
            "Orbot SOCKS Endpoint Not Reachable"
        }
    }

    private func loadPersistedTorState() {
        let persistedUseTor = globalConfig.useTor()
        let persistedMode = TorMode.fromConfig(try? globalConfig.get(key: .torMode))
        let host = (try? globalConfig.get(key: .torExternalHost))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let externalHost = (host?.isEmpty == false) ? host! : "127.0.0.1"
        let externalPort = String(globalConfig.torExternalPort())

        torSettingsDiscovered = globalFlag.getBoolConfig(key: .torSettingsDiscovered) || persistedUseTor
        uiState = TorUiState(
            enabled: persistedUseTor,
            mode: persistedMode,
            status: persistedUseTor ? .bootstrapping : .disabled,
            progressPercent: persistedUseTor ? 1 : 0,
            currentStep: persistedUseTor ? "Loading Tor status" : "Disabled",
            latestLogLine: persistedUseTor ? "Loading Tor status" : "Tor is off",
            logLines: persistedUseTor ? ["[\(torTimestamp())] Loading Tor status"] : ["Tor is off"],
            externalHost: externalHost,
            externalPort: externalPort,
            externalValidationError: validateTorExternalConfig(host: externalHost, port: externalPort),
            orbotStatus: .checking
        )
    }

    private func refreshInitialTorState() async {
        appendTorLog("Opened Tor connection logs")
        if uiState.mode == .orbot {
            await refreshOrbotStatus()
        }

        if uiState.enabled, uiState.mode == .builtIn {
            await ensureBuiltInWarmup()
            syncRustTorLogs()
        }

        await updateNonBuiltInStatus()
        await maybeRunPendingOnionValidation()
    }

    private func handleUseTorToggle(_ enabled: Bool) {
        if !enabled {
            let activeNodeIsOnion = isOnionNodeUrl(app.selectedNode.url)
            let pendingOnionExists = app.pendingNodeAwaitingTorSetup
                && !app.pendingNodeUrl.isEmpty
                && isOnionNodeUrl(app.pendingNodeUrl)

            if activeNodeIsOnion || pendingOnionExists {
                showDisableTorOnionAlert = true
                return
            }

            clearPendingOnionDraft()
            disableTorState()
            appendTorLog("Tor disabled")
            return
        }

        do {
            try globalConfig.setUseTor(useTor: true)
        } catch {
            noticeMessage = "Could not enable Tor: \(error.localizedDescription)"
            appendTorLog("Failed to enable Tor: \(error.localizedDescription)")
            return
        }

        uiState.enabled = true
        uiState.status = .bootstrapping
        uiState.currentStep = "Preparing Tor"
        appendTorLog("Tor enabled")

        if uiState.mode == .builtIn {
            Task { await ensureBuiltInWarmup() }
        }
    }

    private func setTorMode(_ mode: TorMode) {
        do {
            try globalConfig.set(key: .torMode, value: mode.persistedValue)
        } catch {
            noticeMessage = "Could not save Tor mode: \(error.localizedDescription)"
            appendTorLog("Failed to save Tor mode: \(error.localizedDescription)")
            return
        }

        app.pendingNodeTorValidated = false
        autoPendingTestKey = nil
        uiState.mode = mode
        uiState.status = mode == .builtIn ? .bootstrapping : .bootstrapping
        uiState.progressPercent = mode == .builtIn ? 1 : 50
        appendTorLog("Switched Tor mode to \(mode.shortTitle)")

        if mode == .builtIn, uiState.enabled {
            builtInWarmupRequested = false
            Task { await ensureBuiltInWarmup() }
        } else if mode == .orbot {
            Task { await refreshOrbotStatus() }
        }
    }

    private func saveExternalConfig() {
        let validationError = validateTorExternalConfig(host: uiState.externalHost, port: uiState.externalPort)
        if let validationError {
            uiState.externalValidationError = validationError
            noticeMessage = validationError
            return
        }

        do {
            try globalConfig.set(key: .torExternalHost, value: uiState.externalHost)
            if let port = UInt16(uiState.externalPort) {
                try globalConfig.setTorExternalPort(port: port)
            }
        } catch {
            noticeMessage = "Could not save Tor proxy configuration: \(error.localizedDescription)"
            appendTorLog("Failed to save external Tor config: \(error.localizedDescription)")
            return
        }

        app.pendingNodeTorValidated = false
        let proxyLog = redactedProxyForLog(host: uiState.externalHost, port: Int(uiState.externalPort) ?? 0)
        appendTorLog("Saved external Tor config: \(proxyLog)")
        noticeMessage = "Tor proxy configuration saved."
    }

    private func disableTorState() {
        do {
            try globalConfig.setUseTor(useTor: false)
        } catch {
            noticeMessage = "Could not disable Tor: \(error.localizedDescription)"
            appendTorLog("Failed to disable Tor: \(error.localizedDescription)")
            return
        }

        uiState.enabled = false
        uiState.status = .disabled
        uiState.progressPercent = 0
        uiState.currentStep = "Disabled"
        uiState.latestLogLine = "Tor is off"
        builtInWarmupRequested = false
    }

    private func disableTorWithClearnetFallback() async {
        if isOnionNodeUrl(app.selectedNode.url) {
            do {
                let fallback = try await switchToFirstClearnetPresetNode(nodeSelector)
                appendTorLog("Switched active node to \(redactedNodeForLog(fallback)) before disabling Tor")
                noticeMessage = "Switched to \(fallback.name) and disabled Tor."
            } catch {
                appendTorLog("Unable to disable Tor: clearnet fallback failed (\(error.localizedDescription))")
                noticeMessage = "Could not switch to a clearnet node: \(error.localizedDescription)"
                return
            }
        } else if app.pendingNodeAwaitingTorSetup,
                  !app.pendingNodeUrl.isEmpty,
                  isOnionNodeUrl(app.pendingNodeUrl)
        {
            appendTorLog("Discarded pending onion node draft before disabling Tor")
        }

        clearPendingOnionDraft()
        disableTorState()
        appendTorLog("Tor disabled")
    }

    private func clearPendingOnionDraft() {
        app.clearPendingNodeTorDraft()
        autoPendingTestKey = nil
    }

    private func appendTorLog(_ message: String) {
        let entry = "[\(torTimestamp())] \(message)"
        let merged = (uiState.logLines + [entry]).suffix(300)
        uiState.logLines = Array(merged)
        uiState.latestLogLine = message
    }

    private func appendTorTestLog(_ message: String) {
        torTestState.logs = Array((torTestState.logs + [message]).suffix(150))
    }

    private func leadingPercent(_ message: String) -> Int? {
        guard let match = message.range(of: #"^\d{1,3}(?=%:)"#, options: .regularExpression) else {
            return nil
        }
        return Int(message[match]).map { min(max($0, 0), 100) }
    }

    private func syncRustTorLogs() {
        guard uiState.enabled, uiState.mode == .builtIn else { return }

        let rustLogs = torConnectionLogs()
        if rustLogs.count < rustTorLogCount {
            rustTorLogCount = 0
        }

        let newLogs = rustLogs.dropFirst(rustTorLogCount)
        rustTorLogCount = rustLogs.count
        if !newLogs.isEmpty {
            uiState.logLines = Array((uiState.logLines + Array(newLogs)).suffix(300))
        }

        let snapshot = deriveBuiltInBootstrapSnapshot(rustLogs)
        let structuredStatus = builtInTorBootstrapStatus()
        let hasStructuredStatus = structuredStatus.launched
        let nextStatus: TorStatus = if structuredStatus.ready {
            .ready
        } else if structuredStatus.lastError != nil {
            .error
        } else if hasStructuredStatus {
            .bootstrapping
        } else if snapshot.isReady {
            .ready
        } else if snapshot.hasError {
            .error
        } else {
            .bootstrapping
        }
        let currentStep =
            structuredStatus.lastError
                ?? structuredStatus.blocked.map { "Blocked: \($0)" }
                ?? (leadingPercent(snapshot.step) != nil ? snapshot.step : nil)
                ?? (hasStructuredStatus && !structuredStatus.message.isEmpty ? structuredStatus.message : snapshot.step)
        let messagePercent = leadingPercent(currentStep)
        let snapshotPercent = messagePercent ?? (hasStructuredStatus ? Int(structuredStatus.percent) : snapshot.percent)
        uiState.progressPercent = nextStatus == .ready ? 100 : snapshotPercent
        uiState.currentStep = currentStep
        uiState.latestLogLine = snapshot.lastLine
        uiState.status = nextStatus
    }

    private func ensureBuiltInWarmup() async {
        guard !builtInWarmupRequested else { return }
        builtInWarmupRequested = true

        do {
            let endpoint = try await ensureBuiltInTorBootstrap()
            appendTorLog("Built-in Tor bootstrap started at \(endpoint)")
            syncRustTorLogs()
        } catch {
            uiState.status = .error
            appendTorLog("Built-in Tor bootstrap failed: \(error.localizedDescription)")
        }
    }

    private func pollBuiltInTorLogsIfNeeded() async {
        guard uiState.enabled, uiState.mode == .builtIn else { return }

        await ensureBuiltInWarmup()
        while !Task.isCancelled, uiState.enabled, uiState.mode == .builtIn {
            syncRustTorLogs()
            await maybeRunPendingOnionValidation()
            try? await Task.sleep(for: .seconds(1))
        }
    }

    private func refreshOrbotStatus() async {
        uiState.orbotStatus = .checking
        let reachable = await (testSocksEndpoint(host: "127.0.0.1", port: 9050, timeout: 1.5)).isSuccess()
        uiState.orbotStatus = reachable ? .detected : .notDetected
        appendTorLog(reachable ? "Orbot SOCKS endpoint reachable" : "Orbot SOCKS endpoint not reachable")
        await updateNonBuiltInStatus()
    }

    private func updateNonBuiltInStatus() async {
        guard uiState.enabled else {
            uiState.status = .disabled
            uiState.progressPercent = 0
            uiState.currentStep = "Disabled"
            uiState.latestLogLine = "Tor is off"
            return
        }

        switch uiState.mode {
        case .builtIn:
            syncRustTorLogs()
        case .external:
            let validationError = validateTorExternalConfig(host: uiState.externalHost, port: uiState.externalPort)
            uiState.externalValidationError = validationError
            uiState.status = validationError == nil ? .bootstrapping : .error
            uiState.progressPercent = validationError == nil ? 50 : 0
            uiState.currentStep = validationError == nil ? "Save and test custom proxy" : "External proxy config invalid"
            uiState.latestLogLine = validationError
                ?? redactedProxyForLog(host: uiState.externalHost, port: Int(uiState.externalPort) ?? 0)
        case .orbot:
            uiState.status = uiState.orbotStatus == .detected ? .bootstrapping : .error
            uiState.progressPercent = uiState.orbotStatus == .detected ? 50 : 0
            uiState.currentStep = uiState.orbotStatus == .detected ? "Test Orbot connection" : "Start Orbot first"
            uiState.latestLogLine = uiState.orbotStatus == .detected
                ? "Orbot endpoint reachable"
                : "Orbot endpoint unavailable"
        }
    }

    private func testTorProxy(host: String, port: Int) async -> Result<Void, Error> {
        syncRustTorLogs()
        let proxyLog = redactedProxyForLog(host: host, port: port)
        appendTorLog("Testing SOCKS endpoint \(proxyLog)")
        let result = await testSocksEndpoint(host: host, port: port)
        switch result {
        case .success:
            appendTorLog("SOCKS endpoint reachable: \(proxyLog)")
        case let .failure(error):
            appendTorLog("SOCKS endpoint failed: \(proxyLog) (\(error.localizedDescription))")
        }
        syncRustTorLogs()
        return result
    }

    private func resolveNodeForTorTest() async throws -> Node {
        if app.pendingNodeAwaitingTorSetup, !app.pendingNodeUrl.isEmpty {
            let typeName = app.pendingNodeTypeName.isEmpty ? "Custom Electrum" : app.pendingNodeTypeName
            let node = try nodeSelector.parseCustomNode(
                url: app.pendingNodeUrl,
                name: typeName,
                enteredName: app.pendingNodeName
            )
            appendTorLog("Using pending node for Tor test: \(redactedNodeForLog(node))")
            return node
        }

        appendTorLog("Using selected node for Tor test: \(redactedNodeForLog(app.selectedNode))")
        return app.selectedNode
    }

    private func runNodeTorTest(_ node: Node) async -> Result<Void, Error> {
        syncRustTorLogs()
        let nodeLog = redactedNodeForLog(node)
        appendTorLog("Checking node via Tor: \(nodeLog)")
        do {
            try await nodeSelector.checkSelectedNode(node: node)
            appendTorLog("Node check passed: \(nodeLog)")
            syncRustTorLogs()
            return .success(())
        } catch {
            appendTorLog("Node check failed: \(nodeLog) (\(error.localizedDescription))")
            syncRustTorLogs()
            return .failure(error)
        }
    }

    private func updateTorTestStep(_ key: String, status: TorTestStepStatus, detail: String? = nil) {
        torTestState.steps = torTestState.steps.map { step in
            guard step.key == key else { return step }
            var updated = step
            updated.status = status
            if let detail {
                updated.detail = detail
            }
            return updated
        }
    }

    private func runTorApiTestWithRetries(host: String, port: Int) async -> Result<TorApiSnapshot, Error> {
        let maxAttempts = 5
        var timeout: TimeInterval = 15
        var lastError: Error?

        for attempt in 1 ... maxAttempts {
            appendTorTestLog("Tor API check attempt \(attempt)/\(maxAttempts) (timeout=\(Int(timeout * 1000))ms)")
            let result = await testTorApiThroughSocks(host: host, port: port, timeout: timeout)
            if result.isSuccess() {
                return result
            }

            let error = result.failureValue
            lastError = error
            let lowerMessage = error?.localizedDescription.lowercased() ?? ""
            let timeoutLike = lowerMessage.contains("timeout") || lowerMessage.contains("timed out")
            if !timeoutLike || attempt == maxAttempts {
                return .failure(error ?? TorSupportError.timeout)
            }

            let snapshot = deriveBuiltInBootstrapSnapshot(torConnectionLogs())
            appendTorTestLog("Tor API timed out; retrying while Tor bootstraps (\(snapshot.percent)% \(snapshot.step.lowercased()))")
            syncRustTorLogs()
            try? await Task.sleep(for: .milliseconds(min(attempt * 1200, 6000)))
            timeout = min(timeout + 4, 30)
        }

        return .failure(lastError ?? TorSupportError.timeout)
    }

    private func persistTorTestConfiguration(host: String, port: Int) throws {
        try globalConfig.set(key: .torMode, value: uiState.mode.persistedValue)
        if uiState.mode == .external {
            guard let port = UInt16(exactly: port) else {
                throw TorSupportError.invalidEndpoint
            }
            try globalConfig.set(key: .torExternalHost, value: host)
            try globalConfig.setTorExternalPort(port: port)
        }
        try globalConfig.setUseTor(useTor: true)
    }

    private func runProgressiveTorTest() async {
        let endpoint: (String, Int)
        do {
            switch uiState.mode {
            case .builtIn:
                let endpointValue = try await ensureBuiltInTorBootstrap()
                endpoint = parseEndpointHostPort(endpointValue) ?? ("127.0.0.1", 39050)
            case .orbot:
                endpoint = ("127.0.0.1", 9050)
            case .external:
                if let validationError = validateTorExternalConfig(host: uiState.externalHost, port: uiState.externalPort) {
                    uiState.externalValidationError = validationError
                    noticeMessage = validationError
                    return
                }
                endpoint = (uiState.externalHost, Int(uiState.externalPort) ?? 9050)
            }
        } catch {
            app.pendingNodeTorValidated = false
            uiState.status = .error
            appendTorLog("Failed to prepare Tor endpoint: \(error.localizedDescription)")
            noticeMessage = "Failed to prepare Tor endpoint: \(error.localizedDescription)"
            return
        }

        let (host, port) = endpoint
        let proxyLog = redactedProxyForLog(host: host, port: port)

        do {
            try persistTorTestConfiguration(host: host, port: port)
        } catch {
            app.pendingNodeTorValidated = false
            uiState.status = .error
            appendTorLog("Failed to save Tor settings before test: \(error.localizedDescription)")
            noticeMessage = "Could not save Tor settings: \(error.localizedDescription)"
            return
        }

        showTorTestSheet = true
        torTestState = TorConnectionTestState(
            running: true,
            finished: false,
            steps: [
                TorTestStep(key: "proxy", title: "Tor proxy reachable", detail: "Checking SOCKS endpoint \(proxyLog)"),
                TorTestStep(key: "api", title: "Tor API reports Tor exit", detail: "Checking torproject API over SOCKS"),
                TorTestStep(key: "node", title: "Node reachable via Tor", detail: "Checking selected node over Tor"),
            ],
            logs: ["Starting Tor connection test (\(uiState.mode.shortTitle))"]
        )

        updateTorTestStep("proxy", status: .running)
        let proxyResult = await testTorProxy(host: host, port: port)
        if let error = proxyResult.failureValue {
            updateTorTestStep("proxy", status: .failed, detail: "SOCKS check failed: \(error.localizedDescription)")
            appendTorTestLog("SOCKS check failed: \(error.localizedDescription)")
            app.pendingNodeTorValidated = false
            uiState.status = .error
            torTestState.running = false
            torTestState.finished = true
            noticeMessage = "Tor proxy unavailable: \(error.localizedDescription)"
            return
        }
        updateTorTestStep("proxy", status: .passed, detail: "SOCKS endpoint reachable")
        appendTorTestLog("SOCKS endpoint reachable")

        updateTorTestStep("api", status: .running)
        let torApiResult = await runTorApiTestWithRetries(host: host, port: port)
        if let error = torApiResult.failureValue {
            updateTorTestStep("api", status: .failed, detail: "Tor API request failed: \(error.localizedDescription)")
            appendTorTestLog("Tor API request failed: \(error.localizedDescription)")
            app.pendingNodeTorValidated = false
            uiState.status = .error
            torTestState.running = false
            torTestState.finished = true
            noticeMessage = "Tor API check failed: \(error.localizedDescription)"
            return
        }

        guard let apiSnapshot = torApiResult.successValue, apiSnapshot.isTor else {
            let raw = torApiResult.successValue?.raw ?? "missing response"
            updateTorTestStep("api", status: .failed, detail: "Tor API response did not confirm Tor routing")
            appendTorTestLog("Tor API did not confirm Tor routing: \(raw)")
            app.pendingNodeTorValidated = false
            uiState.status = .error
            torTestState.running = false
            torTestState.finished = true
            noticeMessage = "Tor API check failed: traffic is not exiting through Tor."
            return
        }
        let apiDetail = "Tor API confirmed Tor routing\(apiSnapshot.ip.map { " (\($0))" } ?? "")"
        updateTorTestStep("api", status: .passed, detail: apiDetail)
        appendTorTestLog(apiDetail)

        updateTorTestStep("node", status: .running)
        do {
            let node = try await resolveNodeForTorTest()
            let nodeResult = await runNodeTorTest(node)
            if let error = nodeResult.failureValue {
                updateTorTestStep("node", status: .failed, detail: "Node check failed: \(error.localizedDescription)")
                appendTorTestLog("Node check failed: \(error.localizedDescription)")
                app.pendingNodeTorValidated = false
                uiState.status = .error
                torTestState.running = false
                torTestState.finished = true
                noticeMessage = "Node test failed: \(error.localizedDescription)"
                return
            }
        } catch {
            updateTorTestStep("node", status: .failed, detail: "Node parse failed: \(error.localizedDescription)")
            appendTorTestLog("Node parse failed: \(error.localizedDescription)")
            app.pendingNodeTorValidated = false
            uiState.status = .error
            torTestState.running = false
            torTestState.finished = true
            noticeMessage = "Node test failed: \(error.localizedDescription)"
            return
        }

        updateTorTestStep("node", status: .passed, detail: "Node reachable via Tor")
        appendTorTestLog("Node reachable via Tor")
        app.pendingNodeTorValidated = true
        if uiState.mode == .builtIn {
            syncRustTorLogs()
        } else {
            uiState.status = .ready
            uiState.progressPercent = 100
        }
        appendTorLog("\(uiState.mode.shortTitle) validation passed")

        if app.pendingNodeAwaitingTorSetup {
            appendTorTestLog("Pending onion node validated; applying configuration")
            appendTorLog("Pending onion node validated; applying configuration")
            app.popRoute()
        }

        torTestState.running = false
        torTestState.finished = true
        noticeMessage = "Tor connection test passed."
    }

    private func maybeRunPendingOnionValidation() async {
        guard app.pendingNodeAwaitingTorSetup, !app.pendingNodeUrl.isEmpty else {
            autoPendingTestKey = nil
            return
        }
        guard uiState.enabled, !app.pendingNodeTorValidated, !torTestState.running else { return }

        let flowKey = "\(uiState.mode.persistedValue)|\(app.pendingNodeUrl)|\(uiState.externalHost):\(uiState.externalPort)"
        guard autoPendingTestKey != flowKey else { return }

        let readyForAutoTest: Bool
        switch uiState.mode {
        case .builtIn:
            readyForAutoTest = uiState.status == .ready
        case .orbot:
            let socksReady = await (testSocksEndpoint(host: "127.0.0.1", port: 9050, timeout: 1.2)).isSuccess()
            readyForAutoTest = socksReady
        case .external:
            let valid = validateTorExternalConfig(host: uiState.externalHost, port: uiState.externalPort) == nil
            let socksReady = await (testSocksEndpoint(
                host: uiState.externalHost,
                port: Int(uiState.externalPort) ?? 0,
                timeout: 1.2
            )).isSuccess()
            readyForAutoTest = valid && socksReady
        }

        guard readyForAutoTest else { return }
        autoPendingTestKey = flowKey
        appendTorLog("Pending onion node detected; starting automatic Tor validation")
        await runProgressiveTorTest()
    }

    private func openOrbotBestEffort() {
        if let url = URL(string: "orbot://") {
            UIApplication.shared.open(url) { success in
                if !success {
                    appendTorLog("Open Orbot requested; iOS could not open orbot://")
                    noticeMessage = "Open Orbot manually, then return to Cove and refresh status."
                }
            }
            return
        }

        appendTorLog("Open Orbot requested; no iOS URL scheme available")
        noticeMessage = "Open Orbot manually, then return to Cove and refresh status."
    }
}

private struct TorFullLogSheet: View {
    @Environment(\.dismiss) private var dismiss
    let logLines: [String]

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(logLines.enumerated()), id: \.offset) { _, line in
                        Text("> \(line)")
                            .font(.system(.caption2, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .padding()
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(Color(.secondarySystemBackground))
            .navigationTitle("Tor Logs")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button("Copy") {
                        UIPasteboard.general.string = logLines.joined(separator: "\n")
                    }
                }
            }
        }
    }
}

private struct TorTestSheetContent: View {
    let state: TorConnectionTestState
    let onDismiss: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Text("Connection Test")
                .font(.title2.weight(.bold))
                .padding(.top, 20)
                .padding(.bottom, 18)

            VStack(spacing: 16) {
                ForEach(state.steps) { step in
                    TorTestStepProgressRow(step: step)
                }
            }
            .padding(.horizontal, 24)

            Spacer().frame(height: 28)

            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 8) {
                    Image(systemName: "terminal")
                    Text("LIVE TEST LOGS")
                        .font(.caption.weight(.bold))
                        .tracking(0.5)
                }
                .foregroundStyle(.white.opacity(0.7))

                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 3) {
                            ForEach(Array(state.logs.enumerated()), id: \.offset) { index, line in
                                Text(line)
                                    .id(index)
                                    .font(.system(.caption2, design: .monospaced))
                                    .foregroundStyle(.white.opacity(0.9))
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                    }
                    .frame(height: 120)
                    .onChange(of: state.logs.count, initial: true) {
                        if let last = state.logs.indices.last {
                            proxy.scrollTo(last, anchor: .bottom)
                        }
                    }
                }
            }
            .padding(16)
            .background(Color.midnightBlue)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .padding(.horizontal, 24)
            .padding(.top, 8)

            Spacer().frame(height: 28)

            Button(state.running ? "Running Test..." : "Done", action: onDismiss)
                .disabled(state.running)
                .font(.headline)
                .frame(maxWidth: .infinity)
                .padding()
                .background(state.running ? Color.gray.opacity(0.35) : Color.midnightBtn)
                .foregroundStyle(.white)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .padding(.horizontal, 24)
                .padding(.bottom, 28)
        }
        .frame(maxWidth: .infinity)
    }
}

private struct TorTestStepProgressRow: View {
    let step: TorTestStep

    private var statusColor: Color {
        switch step.status {
        case .passed:
            TorStatusDot.green.color
        case .failed:
            TorStatusDot.red.color
        case .running:
            .blue
        case .pending:
            .gray.opacity(0.45)
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            Group {
                switch step.status {
                case .pending:
                    Circle()
                        .stroke(statusColor, lineWidth: 2)
                        .frame(width: 14, height: 14)
                        .padding(5)
                case .running:
                    ProgressView()
                        .frame(width: 24, height: 24)
                case .passed:
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(statusColor)
                        .font(.title3)
                case .failed:
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(statusColor)
                        .font(.title3)
                }
            }
            .frame(width: 24)

            VStack(alignment: .leading, spacing: 3) {
                Text(step.title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(step.status == .pending ? .secondary : .primary)

                if step.status != .pending {
                    Text(step.detail)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
        }
    }
}
