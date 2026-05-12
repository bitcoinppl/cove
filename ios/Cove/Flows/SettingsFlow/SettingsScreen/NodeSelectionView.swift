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
    private var nodeList: [NodeSelection]

    @State private var nodeIsChecking = false
    @State private var customNodeName: String = ""
    @State private var customUrl: String = ""

    @State private var torEnabled: Bool = false
    @State private var torProxyAddress: String = "127.0.0.1:9050"

    @State private var showParseUrlAlert = false
    @State private var parseUrlMessage = ""

    @State private var checkUrlTask: Task<Void, Never>?

    init() {
        let selected = nodeSelector.selectedNode()
        selectedNodeName = selected.name
        nodeList = nodeSelector.nodeList()

        // restore TOR settings from the currently selected node
        if case let .custom(savedNode) = selected {
            _torEnabled = State(initialValue: savedNode.tor.enabled)
            _torProxyAddress = State(initialValue: savedNode.tor.proxyAddress)
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

            Section("Tor Proxy") {
                Toggle("Use Tor", isOn: $torEnabled)
                    .font(.subheadline)

                if torEnabled {
                    HStack {
                        Text("Proxy")
                            .frame(width: 60, alignment: .leading)

                        TextField("127.0.0.1:9050", text: $torProxyAddress)
                            .keyboardType(.numbersAndPunctuation)
                            .textInputAutocapitalization(.never)
                    }
                    .font(.subheadline)

                    Text("Ensure a Tor proxy (e.g. Orbot) is running on this address.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    func checkAndSaveNode() {
        var node: Node? = nil

        let torConfig = TorConfig(
            enabled: torEnabled,
            proxyAddress: torProxyAddress.isEmpty ? "127.0.0.1:9050" : torProxyAddress
        )

        do {
            node = try nodeSelector.parseCustomNodeWithTor(
                url: customUrl,
                name: selectedNodeName,
                enteredName: customNodeName,
                tor: torConfig
            )
            customUrl = node?.url ?? customUrl
            customNodeName = node?.name ?? customNodeName
        } catch {
            showParseUrlAlert = true
            switch error {
            case let NodeSelectorError.ParseNodeUrlError(errorString):
                parseUrlMessage = errorString
            default:
                parseUrlMessage = "Unknown error \(error.localizedDescription)"
            }
        }

        if let node {
            Task {
                showLoadingPopup()
                let result = await Result { try await nodeSelector.checkAndSaveNode(node: node) }

                switch result {
                case .success: completeLoading(.success("Connected to node successfully"))
                case let .failure(error):
                    let errorMessage = "Failed to connect to node\n \(error.localizedDescription)"
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
            if selectedNodeName.hasPrefix("Custom") {
                if case let .custom(savedSelectedNode) = nodeSelector.selectedNode() {
                    if savedSelectedNode.apiType == .electrum, selectedNodeName.contains("Electrum") {
                        customUrl = savedSelectedNode.url
                        customNodeName = savedSelectedNode.name
                        torEnabled = savedSelectedNode.tor.enabled
                        torProxyAddress = savedSelectedNode.tor.proxyAddress
                    }

                    if savedSelectedNode.apiType == .esplora, selectedNodeName.contains("Esplora") {
                        customUrl = savedSelectedNode.url
                        customNodeName = savedSelectedNode.name
                        torEnabled = savedSelectedNode.tor.enabled
                        torProxyAddress = savedSelectedNode.tor.proxyAddress
                    }
                }

                return
            }

            guard let node = try? nodeSelector.selectPresetNode(name: newSelectedNodeName) else { return }

            showLoadingPopup()
            let task = Task {
                do {
                    try await nodeSelector.checkSelectedNode(node: node)
                    completeLoading(.success("Succesfully connected to \(node.url)"))
                } catch {
                    completeLoading(.failure("Failed to connect to \(node.url), reason: \(error.localizedDescription)"))
                }
            }
            checkUrlTask = task
        }
        .onChange(of: nodeList) { _, _ in
            selectedNodeName = nodeSelector.selectedNode().name
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
    }
}

#Preview {
    SettingsContainer(route: .node)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
