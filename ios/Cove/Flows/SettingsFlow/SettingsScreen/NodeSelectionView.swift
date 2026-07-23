//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import MijickPopups
import SwiftUI
import UIKit

private enum DiscoveredNodeAttempt: Equatable {
    case verified(FoundNode)
    case trustRequired(TrustRequiredEndpoint)
}

struct NodeSelectionView: View {
    /// private
    private let nodeSelector = NodeSelector()

    @State private var selectedNodeName: String
    @State private var nodeList: [NodeSelection]
    @State private var nodeScanner = NodeScannerManager()

    @State private var nodeIsChecking = false
    @State private var customNodeName: String = ""
    @State private var customUrl: String = ""

    @State private var showParseUrlAlert = false
    @State private var parseUrlMessage = ""

    @State private var checkUrlTask: Task<Void, Never>?

    @State private var certificateAlert: CertificateDecision?
    @State private var showCertificateAlert = false
    @State private var trustRequiredCandidate: TrustRequiredEndpoint?

    @State private var discoveredNodeAttempt: DiscoveredNodeAttempt?
    @State private var firstScanRetryTask: Task<Void, Never>?
    @State private var nodeScannerIsVisible = false

    @AppStorage("localNodeDiscoveryFirstScanMitigationRan")
    private var firstScanMitigationRan = false

    init() {
        let selectedNode = nodeSelector.selectedNode()

        selectedNodeName = selectedNode.name
        nodeList = nodeSelector.nodeList()

        // These have defaults, so they must be set through their storage rather
        // than assigned, or SwiftUI discards the value when it installs them.
        if case let .custom(node) = selectedNode {
            _customUrl = State(initialValue: node.url)
            _customNodeName = State(initialValue: node.name)
        }
    }

    var certificateAlertTitle: String {
        switch certificateAlert {
        case .changed:
            "Certificate changed"
        case .unrecognized, nil:
            "Unrecognized certificate"
        }
    }

    @ViewBuilder
    private func certificateAlertActions(_ alert: CertificateDecision) -> some View {
        switch alert {
        case let .unrecognized(certificate):
            Button("Trust this certificate") {
                certificateAlert = nil

                if let candidate = trustRequiredCandidate {
                    trustRequiredCandidate = nil
                    connect(to: candidate)
                    return
                }

                checkAndSaveNode(acceptedTls: .pinnedFingerprint(sha256: certificate.sha256))
            }
            Button("Cancel", role: .cancel) {
                certificateAlert = nil
                trustRequiredCandidate = nil
                Task { await dismissAllPopups() }
            }
        case .changed:
            Button("OK", role: .cancel) {
                certificateAlert = nil
                trustRequiredCandidate = nil
            }
        }
    }

    @ViewBuilder
    private func certificateAlertMessage(_ alert: CertificateDecision) -> some View {
        switch alert {
        case let .unrecognized(certificate):
            Text("This server uses a certificate Cove cannot verify. Only continue if this fingerprint matches the one your server reports.\n\n\(certificate.display)")
        case .changed:
            Text("This server is presenting a different certificate to the one you trusted. It may have been reissued, or something may be intercepting the connection. Cove will not connect until it presents the certificate you trusted.")
        }
    }

    var showCustomUrlField: Bool {
        selectedNodeName.hasPrefix("Custom")
    }

    private var discoveredResults: [NodeScannerResult] {
        switch nodeScanner.state {
        case .idle:
            []
        case let .scanning(_, _, _, results),
             let .stopped(_, _, results),
             let .finished(_, _, results),
             let .failed(_, _, _, results):
            results
        }
    }

    private var nodeScanProgress: NodeScanProgress? {
        switch nodeScanner.state {
        case let .scanning(_, _, progress, _):
            progress
        case .idle, .stopped, .finished, .failed:
            nil
        }
    }

    private var nodeScanIsActive: Bool {
        if case .scanning = nodeScanner.state { return true }
        return false
    }

    private var selectedNodeUrl: String {
        nodeSelector.selectedNode().url
    }

    func cancelCheckUrlTask() {
        if let checkUrlTask {
            checkUrlTask.cancel()
        }
    }

    @MainActor
    private func refreshNodeState() {
        let refreshedNodeSelector = NodeSelector()
        nodeList = refreshedNodeSelector.nodeList()
        selectedNodeName = refreshedNodeSelector.selectedNode().name
    }

    private func showLoadingPopup() {
        cancelCheckUrlTask()

        Task { @MainActor in
            await MiddlePopup(state: .loading, onClose: cancelCheckUrlTask)
                .present()
        }
    }

    private func completeLoading(_ state: PopupState) {
        checkUrlTask = nil

        Task { @MainActor in
            await dismissAllPopups()

            let dismissAfter: Double = switch state {
            case .failure:
                7
            case .success:
                2
            default: 0
            }

            try? await Task.sleep(for: .seconds(1))
            await MiddlePopup(state: state)
                .dismissAfter(dismissAfter)
                .present()
        }
    }

    private func toggleNodeScan() {
        cancelFirstScanRetry()

        Task {
            if nodeScanIsActive {
                await nodeScanner.stop()
            } else {
                await nodeScanner.start()
            }
        }
    }

    private func cancelFirstScanRetry() {
        firstScanRetryTask?.cancel()
        firstScanRetryTask = nil
    }

    private func scheduleFirstScanRetryIfNeeded(for state: NodeScannerState) {
        guard !firstScanMitigationRan,
              firstScanRetryTask == nil,
              needsFirstScanRetry(state)
        else {
            return
        }

        let manager = nodeScanner
        firstScanRetryTask = Task { @MainActor [weak manager] in
            do {
                try await Task.sleep(for: .seconds(2))
                try Task.checkCancellation()
            } catch {
                return
            }

            guard nodeScannerIsVisible, let manager else { return }

            firstScanMitigationRan = true
            await manager.start()
            firstScanRetryTask = nil
        }
    }

    private func needsFirstScanRetry(_ state: NodeScannerState) -> Bool {
        switch state {
        case let .finished(_, _, results):
            results.isEmpty
        case let .failed(_, _, error, _):
            error == .LocalNetworkPermissionDenied
        case .idle, .scanning, .stopped:
            false
        }
    }

    private func connect(to found: FoundNode) {
        guard discoveredNodeAttempt == nil else { return }

        discoveredNodeAttempt = .verified(found)
        Task { @MainActor in
            defer { discoveredNodeAttempt = nil }

            do {
                let node = try await nodeScanner.createNode(found)
                await checkAndSaveDiscoveredNode(node)
            } catch {
                presentDiscoveredNodeFailure(error)
            }
        }
    }

    private func connect(to candidate: TrustRequiredEndpoint) {
        guard discoveredNodeAttempt == nil else { return }

        discoveredNodeAttempt = .trustRequired(candidate)
        Task { @MainActor in
            defer { discoveredNodeAttempt = nil }

            do {
                let node = try await nodeScanner.trustAndCreateNode(candidate)
                await checkAndSaveDiscoveredNode(node)
            } catch {
                presentDiscoveredNodeFailure(error)
            }
        }
    }

    @MainActor
    private func checkAndSaveDiscoveredNode(_ node: Node) async {
        showLoadingPopup()

        do {
            try await nodeSelector.checkAndSaveNode(node: node)
            refreshNodeState()
            completeLoading(.success("Connected to \(node.url)"))
        } catch {
            let message = "Failed to connect to \(node.url)\n \(error.localizedDescription)"
            completeLoading(.failure(message))
        }
    }

    private func presentDiscoveredNodeFailure(_ error: Error) {
        let message: String = if let scannerError = error as? NodeScannerError {
            nodeScannerErrorMessage(scannerError)
        } else {
            error.localizedDescription
        }

        completeLoading(.failure("Failed to connect to discovered node\n \(message)"))
    }

    private func nodeScannerErrorMessage(_ error: NodeScannerError) -> String {
        switch error {
        case .NoLocalNetwork:
            "No local network is available"
        case .LocalNetworkPermissionDenied:
            "Local Network access is denied"
        case let .ScanFailed(message):
            message
        case .ResultUnavailable:
            "This result is no longer available. Rescan and try again"
        case .NetworkChanged:
            "The selected Bitcoin network changed. Rescan and try again"
        }
    }

    private func foundNodeUrl(_ node: FoundNode) -> String {
        let scheme = switch node.transport {
        case .tcp:
            "tcp"
        case .tls:
            "ssl"
        }

        return "\(scheme)://\(node.ip):\(node.port)"
    }

    private func foundNodeTransportLabel(_ transport: FoundNodeTransport) -> String {
        switch transport {
        case .tcp:
            "TCP"
        case .tls:
            "TLS"
        }
    }

    private func foundNodeSoftwareLabel(_ node: FoundNode) -> String {
        let software = node.serverSoftware.trimmingCharacters(in: .whitespacesAndNewlines)

        return software.isEmpty ? "Electrum server" : software
    }

    private func presentCertificateAlert(for candidate: TrustRequiredEndpoint) {
        guard discoveredNodeAttempt == nil else { return }

        trustRequiredCandidate = candidate
        certificateAlert = .unrecognized(certificate: candidate.certificate)
        showCertificateAlert = true
    }

    private func openLocalNetworkSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }

        UIApplication.shared.open(url)
    }

    @ViewBuilder
    var CustomFields: some View {
        if showCustomUrlField {
            Section(selectedNodeName) {
                HStack {
                    Text("URL")
                        .frame(width: 60, alignment: .leading)

                    TextField("Enter URL", text: $customUrl)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                }
                .font(.subheadline)

                HStack {
                    Text("Name")
                        .frame(width: 60, alignment: .leading)

                    TextField("Node Name (optional)", text: $customNodeName)
                        .textInputAutocapitalization(.never)
                }
                .font(.subheadline)

                Button("Save Custom Node") {
                    checkAndSaveNode()
                }
                .disabled(customUrl.isEmpty)
            }
        }
    }

    private var LocalNodeDiscoverySection: some View {
        Section {
            if let progress = nodeScanProgress {
                VStack(alignment: .leading, spacing: 8) {
                    ProgressView(
                        value: Double(progress.checkedHosts),
                        total: Double(max(progress.totalHosts, 1))
                    )

                    Text("\(progress.checkedHosts) of \(progress.totalHosts) hosts checked")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
            }

            nodeScannerStatus

            ForEach(discoveredResults, id: \.self) { result in
                discoveredNodeRow(result)
            }
        } header: {
            HStack {
                Text("Found on your network")

                Spacer()

                Button(nodeScanIsActive ? "Stop" : "Rescan", action: toggleNodeScan)
                    .disabled(discoveredNodeAttempt != nil)
                    .textCase(nil)
            }
        }
    }

    @ViewBuilder
    private var nodeScannerStatus: some View {
        switch nodeScanner.state {
        case .idle:
            Text("Cove scans your local network for Electrum servers.")
                .foregroundStyle(.secondary)

        case .scanning:
            EmptyView()

        case let .stopped(_, _, results):
            if results.isEmpty {
                Text("Scan stopped. Rescan to look for Electrum servers.")
                    .foregroundStyle(.secondary)
            }

        case let .finished(_, _, results):
            if results.isEmpty {
                Text("No Electrum servers were found. Make sure this device and your server are on the same Wi-Fi network. Guest networks or client isolation can block local devices, and a VPN can interfere with discovery.")
                    .foregroundStyle(.secondary)
            }

        case let .failed(_, _, error, _):
            nodeScannerFailure(error)
        }
    }

    @ViewBuilder
    private func nodeScannerFailure(_ error: NodeScannerError) -> some View {
        switch error {
        case .LocalNetworkPermissionDenied:
            VStack(alignment: .leading, spacing: 8) {
                Text("Local Network access is needed to find Electrum servers on your Wi-Fi network. Allow access in Settings, then rescan.")
                    .foregroundStyle(.secondary)

                Button("Open Settings", action: openLocalNetworkSettings)
            }

        case .NoLocalNetwork:
            Text("Connect to a private Wi-Fi network to scan for local Electrum servers.")
                .foregroundStyle(.secondary)

        case .ScanFailed:
            Text("Cove couldn't finish scanning the local network. Try again.")
                .foregroundStyle(.secondary)

        case .ResultUnavailable:
            Text("These scan results are no longer available. Rescan to try again.")
                .foregroundStyle(.secondary)

        case .NetworkChanged:
            Text("The selected Bitcoin network changed. Rescan to find compatible servers.")
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private func discoveredNodeRow(_ result: NodeScannerResult) -> some View {
        switch result {
        case let .verified(node):
            verifiedNodeRow(node)
        case let .trustRequired(candidate):
            trustRequiredNodeRow(candidate)
        }
    }

    private func verifiedNodeRow(_ node: FoundNode) -> some View {
        let isSelected = selectedNodeUrl == foundNodeUrl(node)
        let isConnecting = discoveredNodeAttempt == .verified(node)

        return Button {
            connect(to: node)
        } label: {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(foundNodeSoftwareLabel(node))
                        .font(.subheadline)
                        .foregroundStyle(.primary)

                    Text("\(node.ip):\(node.port) • \(foundNodeTransportLabel(node.transport))")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    HStack(spacing: 6) {
                        Text("Verified Electrum server")
                            .foregroundStyle(.green)

                        if isSelected {
                            Text("Selected")
                                .foregroundStyle(.blue)
                                .fontWeight(.semibold)
                        }
                    }
                    .font(.caption)
                }

                Spacer()

                if isConnecting {
                    ProgressView()
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(discoveredNodeAttempt != nil)
        .accessibilityLabel(
            "\(foundNodeSoftwareLabel(node)), \(node.ip):\(node.port), \(foundNodeTransportLabel(node.transport)), verified Electrum server\(isSelected ? ", selected" : "")"
        )
        .accessibilityHint("Connect to this server")
    }

    private func trustRequiredNodeRow(_ candidate: TrustRequiredEndpoint) -> some View {
        let isConnecting = discoveredNodeAttempt == .trustRequired(candidate)

        return Button {
            presentCertificateAlert(for: candidate)
        } label: {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Certificate trust required")
                        .font(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(.orange)

                    Text("\(candidate.ip):\(candidate.port) • TLS")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    Text("Unverified endpoint")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }

                Spacer()

                if isConnecting {
                    ProgressView()
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(discoveredNodeAttempt != nil)
        .accessibilityLabel(
            "Certificate trust required, \(candidate.ip):\(candidate.port), TLS, unverified endpoint"
        )
        .accessibilityHint("Review the certificate fingerprint before connecting")
    }

    func checkAndSaveNode(acceptedTls: TlsTrust? = nil) {
        let tls = acceptedTls
            ?? (selectedNodeName.contains("Electrum")
                ? nodeSelector.trustedCertificate(url: customUrl) : nil)

        let node: Node
        do {
            node = try nodeSelector.parseCustomNode(
                url: customUrl,
                name: selectedNodeName,
                enteredName: customNodeName,
                tls: tls
            )
            customUrl = node.url
            customNodeName = node.name
        } catch let NodeSelectorError.ParseNodeUrlError(errorString) {
            showParseUrlAlert = true
            parseUrlMessage = errorString
            return
        } catch {
            showParseUrlAlert = true
            parseUrlMessage = "Unknown error \(error.localizedDescription)"
            return
        }

        Task {
            showLoadingPopup()

            do {
                try await nodeSelector.checkAndSaveNode(node: node)
                refreshNodeState()
                completeLoading(.success("Connected to node successfully"))
            } catch NodeSelectorError.CertificateNotTrusted {
                // The server is reachable but its certificate was rejected.
                await offerCertificate()
            } catch {
                let errorMessage = "Failed to connect to node\n \(error.localizedDescription)"
                let formattedMessage = errorMessage.replacingOccurrences(of: "\\n", with: "\n")

                completeLoading(.failure(formattedMessage))
            }
        }
    }

    /// Whether the certificate can be offered for confirmation is decided in the
    /// core, so both apps apply the same rule.
    func offerCertificate() async {
        checkUrlTask = nil
        trustRequiredCandidate = nil

        do {
            let decision = try await nodeSelector.certificateDecision(url: customUrl)

            await dismissAllPopups()
            // The popup dismissal is animated, so let it finish before
            // presenting the alert, as the other flows here do.
            try? await Task.sleep(for: .seconds(1))

            certificateAlert = decision
            showCertificateAlert = true
        } catch {
            completeLoading(.failure("Could not read the server's certificate\n \(error.localizedDescription)"))
        }
    }

    var body: some View {
        Form {
            Section {
                ForEach(nodeList, id: \.name) { (node: NodeSelection) in
                    HStack {
                        Text(node.name)
                            .font(.subheadline)

                        Spacer()

                        if selectedNodeName == node.name {
                            Image(systemName: "checkmark")
                                .foregroundStyle(.blue)
                                .font(.footnote)
                                .fontWeight(.semibold)
                        }
                    }
                    .contentShape(Rectangle())
                    .onTapGesture { selectedNodeName = node.name }
                }

                HStack {
                    Text("Custom Electrum")
                        .font(.subheadline)

                    Spacer()

                    if selectedNodeName == "Custom Electrum" {
                        Image(systemName: "checkmark")
                            .foregroundStyle(.blue)
                            .font(.footnote)
                            .fontWeight(.semibold)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture { selectedNodeName = "Custom Electrum" }

                HStack {
                    Text("Custom Esplora")
                        .font(.subheadline)

                    Spacer()

                    if selectedNodeName == "Custom Esplora" {
                        Image(systemName: "checkmark")
                            .foregroundStyle(.blue)
                            .font(.footnote)
                            .fontWeight(.semibold)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture { selectedNodeName = "Custom Esplora" }
            }

            LocalNodeDiscoverySection

            CustomFields
        }
        .scrollContentBackground(.hidden)
        .task {
            nodeScannerIsVisible = true

            await withTaskCancellationHandler {
                await nodeScanner.start()
            } onCancel: {
                Task { await nodeScanner.stop() }
            }
        }
        .onChange(of: nodeScanner.state) { _, state in
            scheduleFirstScanRetryIfNeeded(for: state)
        }
        .onChange(of: selectedNodeName) { _, newSelectedNodeName in
            guard nodeSelector.selectedNode().name != newSelectedNodeName else { return }

            if selectedNodeName.hasPrefix("Custom") {
                if case let .custom(savedSelectedNode) = nodeSelector.selectedNode() {
                    if savedSelectedNode.apiType == .electrum, selectedNodeName.contains("Electrum") {
                        customUrl = savedSelectedNode.url
                        customNodeName = savedSelectedNode.name
                    }

                    if savedSelectedNode.apiType == .esplora, selectedNodeName.contains("Esplora") {
                        customUrl = savedSelectedNode.url
                        customNodeName = savedSelectedNode.name
                    }
                }

                return
            }

            guard let node = try? nodeSelector.selectPresetNode(name: newSelectedNodeName) else { return }

            showLoadingPopup()
            let task = Task {
                do {
                    try await nodeSelector.checkSelectedNode(node: node)
                    refreshNodeState()
                    completeLoading(.success("Succesfully connected to \(node.url)"))
                } catch {
                    completeLoading(.failure("Failed to connect to \(node.url), reason: \(error.localizedDescription)"))
                }
            }
            checkUrlTask = task
        }
        .onDisappear {
            nodeScannerIsVisible = false
            cancelFirstScanRetry()
            Task { await nodeScanner.stop() }

            // custom esplora or electrum is selected
            if showCustomUrlField { checkAndSaveNode() }
        }
        .alert(isPresented: $showParseUrlAlert) {
            Alert(
                title: Text("Unable to parse URL"),
                message: Text(parseUrlMessage),
                dismissButton: .default(Text("OK")) {
                    showParseUrlAlert = false
                    parseUrlMessage = ""
                    Task { await dismissAllPopups() }
                }
            )
        }
        .alert(
            certificateAlertTitle,
            isPresented: $showCertificateAlert,
            presenting: certificateAlert,
            actions: certificateAlertActions,
            message: certificateAlertMessage
        )
    }
}

#Preview {
    SettingsContainer(route: .node)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
