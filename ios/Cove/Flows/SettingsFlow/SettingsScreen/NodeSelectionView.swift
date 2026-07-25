//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import MijickPopups
import SwiftUI

private enum NodeSelectionAlert: Identifiable {
    case message(title: String, body: String)
    case onionNodeRequiresTor
    case apiTypeMismatch(selected: ApiType, inferred: ApiType)
    case torEnableError

    /// content-derived so a second failure with a different message still re-presents
    var id: String {
        switch self {
        case let .message(title, body): "message:\(title):\(body)"
        case .onionNodeRequiresTor: "onionNodeRequiresTor"
        case let .apiTypeMismatch(selected, inferred): "apiTypeMismatch:\(selected):\(inferred)"
        case .torEnableError: "torEnableError"
        }
    }
}

struct NodeSelectionView: View {
    @Environment(AppManager.self) private var app

    /// private
    private let nodeSelector = NodeSelector()

    @State private var selectedNodeName: String
    @State private var nodeList: [NodeSelection]

    @State private var nodeIsChecking = false
    @State private var customNodeName: String = ""
    @State private var customUrl: String = ""

    @State private var nodeAlert: NodeSelectionAlert?
    @State private var navigatingToTorSettings = false

    @State private var checkUrlTask: Task<Void, Never>?

    init() {
        selectedNodeName = nodeSelector.selectedNode().name
        nodeList = nodeSelector.nodeList()
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

                Button("Save Custom Node", action: checkAndSaveNode)
                    .disabled(customUrl.isEmpty)
            }
        }
    }

    func checkAndSaveNode() {
        var node: Node? = nil

        do {
            node = try nodeSelector.parseCustomNode(url: customUrl, name: selectedNodeName, enteredName: customNodeName)
            customUrl = node?.url ?? customUrl
            customNodeName = node?.name ?? customNodeName
        } catch {
            nodeAlert = nodeSelectionAlert(for: error)
        }

        if let node {
            let torWasEnabled = TorManager.shared.isEnabled

            Task {
                showLoadingPopup()
                let result = await Result { try await nodeSelector.checkAndSaveNode(node: node) }

                switch result {
                case .success:
                    refreshNodeState()

                    // saving an onion node turns Tor on behind the user's back, so say so
                    let torWasEnabledForNode = !torWasEnabled && TorManager.shared.isEnabled
                    let message = torWasEnabledForNode
                        ? "Connected to node successfully\nBuilt-in Tor was enabled to reach this node."
                        : "Connected to node successfully"

                    completeLoading(.success(message))
                case let .failure(error):
                    if let alert = typedNodeSelectionAlert(for: error) {
                        await presentNodeSelectionAlert(alert)
                        return
                    }

                    let errorMessage = "Failed to connect to node\n\(nodeErrorMessage(error))"
                    let formattedMessage = errorMessage.replacingOccurrences(of: "\\n", with: "\n")

                    completeLoading(.failure(formattedMessage))
                }
            }
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

            CustomFields
        }
        .scrollContentBackground(.hidden)
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
                    if let alert = typedNodeSelectionAlert(for: error) {
                        await presentNodeSelectionAlert(alert)
                        return
                    }

                    completeLoading(.failure("Failed to connect to \(node.url)\n\(nodeErrorMessage(error))"))
                }
            }
            checkUrlTask = task
        }
        .onAppear {
            navigatingToTorSettings = false
        }
        .onDisappear {
            guard !navigatingToTorSettings else { return }

            // custom esplora or electrum is selected
            if showCustomUrlField { checkAndSaveNode() }
        }
        .alert(item: $nodeAlert) { alert in
            switch alert {
            case let .message(title, body):
                Alert(
                    title: Text(title),
                    message: Text(body),
                    dismissButton: .default(Text("OK")) {
                        Task { await dismissAllPopups() }
                    }
                )

            case .onionNodeRequiresTor:
                torSetupAlert(
                    message: "This onion node requires Tor. Set up Tor, then return and save the node again."
                )

            case let .apiTypeMismatch(selected, inferred):
                Alert(
                    title: Text("Node Type Does Not Match"),
                    message: Text(
                        "This URL uses \(apiTypeName(inferred)), but \(apiTypeName(selected)) is selected. Choose the matching custom node type or change the URL scheme."
                    ),
                    dismissButton: .default(Text("OK"))
                )

            case .torEnableError:
                torSetupAlert(
                    title: "Unable to Enable Tor",
                    message: "Cove could not enable Tor for this onion node. Review Tor settings, then return and save the node again."
                )
            }
        }
    }

    private func typedNodeSelectionAlert(for error: Error) -> NodeSelectionAlert? {
        guard let error = error as? NodeSelectorError else { return nil }

        return switch error {
        case .OnionNodeRequiresTor:
            .onionNodeRequiresTor
        case let .ApiTypeMismatch(selected, inferred):
            .apiTypeMismatch(selected: selected, inferred: inferred)
        case .TorEnableError:
            .torEnableError
        case .NodeNotFound,
             .SetSelectedNodeError,
             .NodeAccessError,
             .ParseNodeUrlError,
             .TorSettingsDiscoveryError:
            nil
        }
    }

    private func nodeSelectionAlert(for error: Error) -> NodeSelectionAlert {
        if let alert = typedNodeSelectionAlert(for: error) {
            return alert
        }

        if let error = error as? NodeSelectorError,
           case .ParseNodeUrlError = error
        {
            return .message(title: "Unable to Parse URL", body: nodeErrorMessage(error))
        }

        return .message(title: "Unable to Save Node", body: nodeErrorMessage(error))
    }

    /// `NodeSelectorError` has no exported `Display`, and its synthesized `errorDescription` is a
    /// debug dump, so every case gets its own user-facing wording here
    private func nodeErrorMessage(_ error: Error) -> String {
        guard let error = error as? NodeSelectorError else { return error.localizedDescription }

        return switch error {
        case let .NodeNotFound(name):
            "Cove has no node named \(name)."
        case let .SetSelectedNodeError(reason):
            "Cove could not save this node: \(reason)"
        case let .NodeAccessError(reason):
            "Cove could not reach this node: \(reason)"
        case let .ParseNodeUrlError(reason):
            "This is not a valid node address: \(reason)"
        case .OnionNodeRequiresTor:
            "This node uses an onion address, which can only be reached over Tor."
        case let .ApiTypeMismatch(selected, inferred):
            "This URL uses \(apiTypeName(inferred)), but \(apiTypeName(selected)) is selected."
        case .TorEnableError:
            "Cove could not enable Tor for this onion node."
        case .TorSettingsDiscoveryError:
            "Cove could not save your Tor settings."
        }
    }

    @MainActor
    private func presentNodeSelectionAlert(_ alert: NodeSelectionAlert) async {
        checkUrlTask = nil
        await dismissAllPopups()
        nodeAlert = alert
    }

    private func torSetupAlert(
        title: String = "Tor Required",
        message: String
    ) -> Alert {
        Alert(
            title: Text(title),
            message: Text(message),
            primaryButton: .default(Text("Set Up Tor")) {
                navigatingToTorSettings = true
                app.pushRoute(.settings(.tor))
            },
            secondaryButton: .cancel()
        )
    }

    private func apiTypeName(_ apiType: ApiType) -> String {
        switch apiType {
        case .electrum:
            "Electrum"
        case .esplora:
            "Esplora"
        case .rpc:
            "Bitcoin RPC"
        }
    }
}

#Preview {
    SettingsContainer(route: .node)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
