//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import MijickPopups
import SwiftUI

private struct CustomNodeRequest: Equatable {
    let requestedSelectionName: String
    let node: Node
}

private struct PresetNodeRequest: Equatable {
    let id = UUID()
    let name: String
}

private struct PendingCertificate {
    let request: CustomNodeRequest
    let decision: CertificateDecision
}

struct NodeSelectionView: View {
    /// private
    private let nodeSelector = NodeSelector()

    @State private var selectedNodeName: String
    @State private var nodeList: [NodeSelection]
    @State private var selectedNodeState: NodeRuntimeState

    @State private var nodeIsChecking = false
    @State private var customNodeName: String = ""
    @State private var customUrl: String = ""

    @State private var showParseUrlAlert = false
    @State private var parseUrlMessage = ""

    @State private var checkUrlTask: Task<Void, Never>?
    @State private var presetNodeRequest: PresetNodeRequest?

    /// A certificate accepted in this session, paired with its endpoint
    /// `parseCustomNode` decides whether the endpoint matches the request
    @State private var customCertificateTrust: EndpointCertificateTrust?
    @State private var certificateAlert: PendingCertificate?
    @State private var showCertificateAlert = false

    init() {
        let selectedNodeState = nodeSelector.selectedNode()
        let selectedNode = selectedNodeState.storedNode()

        selectedNodeName = selectedNode.name
        nodeList = nodeSelector.nodeList()
        _selectedNodeState = State(initialValue: selectedNodeState)

        // These have defaults, so they must be set through their storage rather
        // than assigned, or SwiftUI discards the value when it installs them.
        if case let .custom(node) = selectedNodeState.storedSelection() {
            _customUrl = State(initialValue: node.url)
            _customNodeName = State(initialValue: node.name)

            if let tls = node.tls {
                _customCertificateTrust = State(
                    initialValue: EndpointCertificateTrust(endpoint: node.url, tls: tls)
                )
            }
        }
    }

    var showCustomUrlField: Bool {
        selectedNodeName.hasPrefix("Custom")
    }

    func cancelCheckUrlTask() {
        presetNodeRequest = nil
        nodeSelector.cancelPendingSelection()
        checkUrlTask?.cancel()
        checkUrlTask = nil
    }

    @MainActor
    private func refreshNodeState() {
        let refreshedNodeSelector = NodeSelector()
        nodeList = refreshedNodeSelector.nodeList()
        selectedNodeState = refreshedNodeSelector.selectedNode()
        selectedNodeName = selectedNodeState.storedNode().name
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
        presetNodeRequest = nil

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

    func checkAndSaveCustomNode() {
        guard !nodeIsChecking, !showCertificateAlert, certificateAlert == nil else { return }

        let sessionTrust = customCertificateTrust
        let node: Node
        do {
            node = try nodeSelector.parseCustomNode(
                url: customUrl,
                name: selectedNodeName,
                enteredName: customNodeName,
                certificateTrust: sessionTrust
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

        let request = CustomNodeRequest(requestedSelectionName: selectedNodeName, node: node)

        nodeIsChecking = true
        showLoadingPopup()
        Task { @MainActor in
            defer { nodeIsChecking = false }

            do {
                try await nodeSelector.checkNode(node: node)

                guard isCurrentCustomSelection(request: request) else {
                    await dismissAllPopups()
                    return
                }

                try await nodeSelector.saveNode(node: node)
                refreshNodeState()
                completeLoading(.success("Connected to node successfully"))
            } catch NodeSelectorError.CertificateNotTrusted {
                // The server is reachable but its certificate was rejected.
                guard isCurrentCustomSelection(request: request) else {
                    await dismissAllPopups()
                    return
                }

                await offerCertificate(request: request)
            } catch {
                guard isCurrentCustomSelection(request: request) else {
                    await dismissAllPopups()
                    return
                }

                let errorMessage = "Failed to connect to node\n \(error.localizedDescription)"
                let formattedMessage = errorMessage.replacingOccurrences(of: "\\n", with: "\n")

                completeLoading(.failure(formattedMessage))
            }
        }
    }

    /// Whether the certificate can be offered for confirmation is decided in the
    /// core, so both apps apply the same rule.
    private func offerCertificate(request: CustomNodeRequest) async {
        checkUrlTask = nil

        do {
            let decision = try await nodeSelector.certificateDecision(url: request.node.url)

            guard isCurrentCustomSelection(request: request) else {
                await dismissAllPopups()
                return
            }

            await dismissAllPopups()
            // The popup dismissal is animated, so let it finish before
            // presenting the alert, as the other flows here do.
            try? await Task.sleep(for: .seconds(1))

            guard isCurrentCustomSelection(request: request) else { return }

            certificateAlert = PendingCertificate(request: request, decision: decision)
            showCertificateAlert = true
        } catch {
            guard isCurrentCustomSelection(request: request) else {
                await dismissAllPopups()
                return
            }

            completeLoading(.failure("Could not read the server's certificate\n \(error.localizedDescription)"))
        }
    }

    private func isCurrentCustomSelection(request: CustomNodeRequest) -> Bool {
        guard selectedNodeName == request.requestedSelectionName else { return false }

        do {
            let currentNode = try nodeSelector.parseCustomNode(
                url: customUrl,
                name: selectedNodeName,
                enteredName: customNodeName,
                certificateTrust: customCertificateTrust
            )

            return currentNode == request.node
        } catch {
            return false
        }
    }

    var body: some View {
        NodeSelectionForm(
            nodeList: nodeList,
            selectedNodeState: selectedNodeState,
            selectedNodeName: $selectedNodeName,
            customUrl: $customUrl,
            customNodeName: $customNodeName,
            saveCustomNode: checkAndSaveCustomNode
        )
        .scrollContentBackground(.hidden)
        .disabled(nodeIsChecking)
        .onChange(of: selectedNodeName) { _, newSelectedNodeName in
            nodeSelectionChanged(to: newSelectedNodeName)
        }
        .onDisappear {
            // custom esplora or electrum is selected
            guard showCustomUrlField, !nodeIsChecking, !showCertificateAlert,
                  certificateAlert == nil
            else { return }

            checkAndSaveCustomNode()
        }
        .modifier(
            NodeSelectionAlertsModifier(
                showParseUrlAlert: $showParseUrlAlert,
                parseUrlMessage: parseUrlMessage,
                showCertificateAlert: $showCertificateAlert,
                certificateAlert: $certificateAlert,
                dismissParseUrlAlert: dismissParseUrlAlert,
                trustCertificate: trustCertificate,
                cancelCertificateAlert: cancelCertificateAlert
            )
        )
    }

    private func dismissParseUrlAlert() {
        showParseUrlAlert = false
        parseUrlMessage = ""
        Task { await dismissAllPopups() }
    }

    private func trustCertificate(_ pending: PendingCertificate) {
        guard case let .unrecognized(certificate) = pending.decision,
              isCurrentCustomSelection(request: pending.request)
        else {
            certificateAlert = nil
            showCertificateAlert = false
            return
        }

        certificateAlert = nil
        showCertificateAlert = false
        customCertificateTrust = EndpointCertificateTrust(
            endpoint: pending.request.node.url,
            tls: .pinnedFingerprint(sha256: certificate.sha256)
        )
        customUrl = pending.request.node.url
        checkAndSaveCustomNode()
    }

    private func cancelCertificateAlert() {
        certificateAlert = nil
        Task { await dismissAllPopups() }
    }

    private func nodeSelectionChanged(to newSelectedNodeName: String) {
        cancelCheckUrlTask()

        guard selectedNodeState.storedNode().name != newSelectedNodeName else { return }

        if newSelectedNodeName.hasPrefix("Custom") {
            restoreCustomNodeFields(for: newSelectedNodeName)
            return
        }

        showLoadingPopup()
        let request = PresetNodeRequest(name: newSelectedNodeName)
        presetNodeRequest = request
        checkUrlTask = Task {
            do {
                let node = try await nodeSelector.selectPresetNode(name: newSelectedNodeName)

                guard isCurrentPresetSelection(request: request) else {
                    await dismissAllPopups()
                    return
                }

                refreshNodeState()
                completeLoading(.success("Successfully connected to \(node.url)"))
            } catch {
                guard isCurrentPresetSelection(request: request) else {
                    await dismissAllPopups()
                    return
                }

                selectedNodeState = nodeSelector.selectedNode()
                selectedNodeName = selectedNodeState.storedNode().name
                completeLoading(.failure("Failed to select \(newSelectedNodeName), reason: \(error.localizedDescription)"))
            }
        }
    }

    private func isCurrentPresetSelection(request: PresetNodeRequest) -> Bool {
        presetNodeRequest == request && selectedNodeName == request.name
    }

    private func restoreCustomNodeFields(for selectedNodeName: String) {
        guard case let .custom(savedSelectedNode) = selectedNodeState.storedSelection() else {
            customCertificateTrust = nil
            return
        }

        let matchesApiType =
            savedSelectedNode.apiType == .electrum && selectedNodeName.contains("Electrum")
                || savedSelectedNode.apiType == .esplora && selectedNodeName.contains("Esplora")
        guard matchesApiType else {
            customCertificateTrust = nil
            return
        }

        customUrl = savedSelectedNode.url
        customNodeName = savedSelectedNode.name
        customCertificateTrust = savedSelectedNode.tls.map { tls in
            EndpointCertificateTrust(endpoint: savedSelectedNode.url, tls: tls)
        }
    }
}

private struct NodeSelectionAlertsModifier: ViewModifier {
    @Binding var showParseUrlAlert: Bool
    let parseUrlMessage: String
    @Binding var showCertificateAlert: Bool
    @Binding var certificateAlert: PendingCertificate?
    let dismissParseUrlAlert: () -> Void
    let trustCertificate: (PendingCertificate) -> Void
    let cancelCertificateAlert: () -> Void

    private var certificateAlertTitle: String {
        switch certificateAlert?.decision {
        case .changed: "Certificate changed"
        default: "Unrecognized certificate"
        }
    }

    func body(content: Content) -> some View {
        content
            .alert(isPresented: $showParseUrlAlert) {
                Alert(
                    title: Text("Unable to parse URL"),
                    message: Text(parseUrlMessage),
                    dismissButton: .default(Text("OK"), action: dismissParseUrlAlert)
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

    @ViewBuilder
    private func certificateAlertActions(_ pending: PendingCertificate) -> some View {
        switch pending.decision {
        case .unrecognized:
            Button("Trust this certificate") {
                trustCertificate(pending)
            }
            Button("Cancel", role: .cancel, action: cancelCertificateAlert)
        case .changed:
            Button("OK", role: .cancel) { certificateAlert = nil }
        }
    }

    @ViewBuilder
    private func certificateAlertMessage(_ pending: PendingCertificate) -> some View {
        switch pending.decision {
        case let .unrecognized(certificate):
            Text("This server uses a certificate Cove cannot verify. Only continue if this fingerprint matches the one your server reports.\n\n\(certificate.display)")
        case .changed:
            Text("This server is presenting a different certificate to the one you trusted. It may have been reissued, or something may be intercepting the connection. Cove will not connect until it presents the certificate you trusted.")
        }
    }
}

private struct NodeSelectionForm: View {
    let nodeList: [NodeSelection]
    let selectedNodeState: NodeRuntimeState
    @Binding var selectedNodeName: String
    @Binding var customUrl: String
    @Binding var customNodeName: String
    let saveCustomNode: () -> Void

    var body: some View {
        Form {
            NodeRuntimeWarning(state: selectedNodeState)
            NodeSelectionPresetSection(
                nodeList: nodeList,
                selectedNodeName: $selectedNodeName
            )
            NodeSelectionCustomFields(
                selectedNodeName: selectedNodeName,
                customUrl: $customUrl,
                customNodeName: $customNodeName,
                save: saveCustomNode
            )
        }
    }
}

private struct NodeRuntimeWarning: View {
    let state: NodeRuntimeState

    var body: some View {
        if case let .fallback(_, runtimeSelection, reason) = state {
            Section {
                Label {
                    Text(warningText(runtimeSelection: runtimeSelection, reason: reason))
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                }
            }
        }
    }

    private func warningText(
        runtimeSelection: NodeSelection,
        reason: NodeRuntimeFallbackReason
    ) -> String {
        let runtimeNode = runtimeSelection.toNode()
        let reasonText = switch reason {
        case .invalidTrustStorage:
            "the saved certificate trust data is invalid"
        case .endpointConflict:
            "certificate trust for the saved endpoint conflicts"
        }

        return "Cove is using \(runtimeNode.name) (\(runtimeNode.url)) because \(reasonText). Your saved node remains selected until you choose a replacement."
    }
}

private struct NodeSelectionPresetSection: View {
    let nodeList: [NodeSelection]
    @Binding var selectedNodeName: String

    var body: some View {
        Section {
            ForEach(nodeList, id: \.name) { node in
                NodeSelectionRow(
                    name: node.name,
                    isSelected: selectedNodeName == node.name,
                    select: { selectedNodeName = node.name }
                )
            }
            NodeSelectionRow(
                name: "Custom Electrum",
                isSelected: selectedNodeName == "Custom Electrum",
                select: { selectedNodeName = "Custom Electrum" }
            )
            NodeSelectionRow(
                name: "Custom Esplora",
                isSelected: selectedNodeName == "Custom Esplora",
                select: { selectedNodeName = "Custom Esplora" }
            )
        }
    }
}

private struct NodeSelectionRow: View {
    let name: String
    let isSelected: Bool
    let select: () -> Void

    var body: some View {
        HStack {
            Text(name)
                .font(.subheadline)

            Spacer()

            if isSelected {
                Image(systemName: "checkmark")
                    .foregroundStyle(.blue)
                    .font(.footnote)
                    .fontWeight(.semibold)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: select)
    }
}

private struct NodeSelectionCustomFields: View {
    let selectedNodeName: String
    @Binding var customUrl: String
    @Binding var customNodeName: String
    let save: () -> Void

    var body: some View {
        if selectedNodeName.hasPrefix("Custom") {
            Section(selectedNodeName) {
                NodeSelectionTextField(
                    title: "URL",
                    placeholder: "Enter URL",
                    text: $customUrl,
                    isUrl: true
                )
                NodeSelectionTextField(
                    title: "Name",
                    placeholder: "Node Name (optional)",
                    text: $customNodeName,
                    isUrl: false
                )
                Button("Save Custom Node", action: save)
                    .disabled(customUrl.isEmpty)
            }
        }
    }
}

private struct NodeSelectionTextField: View {
    let title: String
    let placeholder: String
    @Binding var text: String
    let isUrl: Bool

    var body: some View {
        HStack {
            Text(title)
                .frame(width: 60, alignment: .leading)

            TextField(placeholder, text: $text)
                .keyboardType(isUrl ? .URL : .default)
                .textInputAutocapitalization(.never)
        }
        .font(.subheadline)
    }
}

#Preview {
    SettingsContainer(route: .node)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
