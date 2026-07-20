//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import MijickPopups
import SwiftUI

struct NodeSelectionView: View {
    /// private
    private let nodeSelector = NodeSelector()

    @State private var selectedNodeName: String
    @State private var nodeList: [NodeSelection]

    @State private var nodeIsChecking = false
    @State private var customNodeName: String = ""
    @State private var customUrl: String = ""

    @State private var showParseUrlAlert = false
    @State private var parseUrlMessage = ""

    @State private var checkUrlTask: Task<Void, Never>?

    @State private var customTls: TlsTrust?
    @State private var certificateAlert: CertificateDecision?
    @State private var showCertificateAlert = false

    init() {
        let selectedNode = nodeSelector.selectedNode()

        selectedNodeName = selectedNode.name
        nodeList = nodeSelector.nodeList()

        // Carry the saved node's certificate settings, so saving it again does
        // not fall back to default trust and ask about the certificate afresh.
        // These have defaults, so they must be set through their storage rather
        // than assigned, or SwiftUI discards the value when it installs them.
        if case let .custom(node) = selectedNode {
            _customUrl = State(initialValue: node.url)
            _customNodeName = State(initialValue: node.name)
            _customTls = State(initialValue: node.tls)
        }
    }

    /// Whether the custom fields differ from the node that is already saved.
    var hasUnsavedCustomNode: Bool {
        guard case let .custom(saved) = nodeSelector.selectedNode() else { return !customUrl.isEmpty }
        return saved.url != customUrl || saved.name != customNodeName
    }

    var certificateAlertTitle: String {
        switch certificateAlert {
        case .changed: "Certificate changed"
        default: "Unrecognized certificate"
        }
    }

    @ViewBuilder
    private func certificateAlertActions(_ alert: CertificateDecision) -> some View {
        switch alert {
        case let .unrecognized(certificate):
            Button("Trust this certificate") {
                certificateAlert = nil
                customTls = .pinnedFingerprint(sha256: certificate.sha256)
                checkAndSaveNode()
            }
            Button("Cancel", role: .cancel) {
                certificateAlert = nil
                Task { await dismissAllPopups() }
            }
        case .changed:
            Button("OK", role: .cancel) { certificateAlert = nil }
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

    func checkAndSaveNode() {
        let node: Node
        do {
            node = try nodeSelector.parseCustomNode(
                url: customUrl,
                name: selectedNodeName,
                enteredName: customNodeName,
                tls: customTls
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
        NodeSelectionForm(
            nodeList: nodeList,
            selectedNodeName: $selectedNodeName,
            customUrl: $customUrl,
            customNodeName: $customNodeName,
            saveCustomNode: checkAndSaveNode
        )
        .scrollContentBackground(.hidden)
        .onChange(of: selectedNodeName) { _, newSelectedNodeName in
            nodeSelectionChanged(to: newSelectedNodeName)
        }
        .onDisappear {
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

    private func nodeSelectionChanged(to newSelectedNodeName: String) {
        guard nodeSelector.selectedNode().name != newSelectedNodeName else { return }

        if newSelectedNodeName.hasPrefix("Custom") {
            restoreCustomNodeFields(for: newSelectedNodeName)
            return
        }

        guard let node = try? nodeSelector.selectPresetNode(name: newSelectedNodeName) else { return }

        showLoadingPopup()
        checkUrlTask = Task {
            do {
                try await nodeSelector.checkSelectedNode(node: node)
                refreshNodeState()
                completeLoading(.success("Succesfully connected to \(node.url)"))
            } catch {
                completeLoading(.failure("Failed to connect to \(node.url), reason: \(error.localizedDescription)"))
            }
        }
    }

    private func restoreCustomNodeFields(for selectedNodeName: String) {
        guard case let .custom(savedSelectedNode) = nodeSelector.selectedNode() else { return }

        let matchesApiType =
            savedSelectedNode.apiType == .electrum && selectedNodeName.contains("Electrum")
                || savedSelectedNode.apiType == .esplora && selectedNodeName.contains("Esplora")
        guard matchesApiType else { return }

        customUrl = savedSelectedNode.url
        customNodeName = savedSelectedNode.name
        customTls = savedSelectedNode.tls
    }
}

private struct NodeSelectionForm: View {
    let nodeList: [NodeSelection]
    @Binding var selectedNodeName: String
    @Binding var customUrl: String
    @Binding var customNodeName: String
    let saveCustomNode: () -> Void

    var body: some View {
        Form {
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
