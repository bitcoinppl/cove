//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import MijickPopupView
import SwiftUI

struct NodeSelectionView: View {
    // private
    private let nodeSelector = NodeSelector()

    @State private var selectedNodeName: String
    private var nodeList: [NodeSelection]

    @State private var nodeIsChecking = false
    @State private var customNodeName: String = ""
    @State private var customUrl: String = ""

    @State private var showParseUrlAlert = false
    @State private var parseUrlMessage = ""

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

    private func showLoadingPopup() {
        cancelCheckUrlTask()

        MiddlePopup(state: .loading, onClose: cancelCheckUrlTask)
            .showAndStack()
    }

    private func completeLoading(_ state: PopupState) {
        checkUrlTask = nil
        PopupManager.dismiss()

        let dismissAfter: Double = switch state {
        case .failure:
            7
        case .success:
            2
        default: 0
        }

        Task {
            try? await Task.sleep(for: .seconds(1))
            await MainActor.run {
                let _ = MiddlePopup(state: state)
                    .showAndReplace()
                    .dismissAfter(dismissAfter)
            }
        }
    }

    @ViewBuilder
    var CustomFields: some View {
        TextField("Enter \(selectedNodeName) URL", text: $customUrl)
            .keyboardType(.URL)
            .textInputAutocapitalization(.never)

        TextField("Enter Node Name (optional)", text: $customNodeName)
            .textInputAutocapitalization(.never)

        Button("Save \(selectedNodeName)") {
            var node: Node? = nil
            do {
                node = try nodeSelector.parseCustomNode(url: customUrl, name: selectedNodeName, enteredName: customNodeName)
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
        .disabled(customUrl.isEmpty)
    }

    var body: some View {
        Section(header: Text("Node Selection")) {
            Picker("Select Node", selection: $selectedNodeName) {
                ForEach(nodeList, id: \.name) { (node: NodeSelection) in
                    Text(node.name)
                        .tag(node.name)
                }

                Text("Custom Electrum").tag("Custom Electrum")
                Text("Custom Esplora").tag("Custom Esplora")
            }

            if showCustomUrlField {
                CustomFields
            }
        }
        .onChange(of: selectedNodeName) { _, newSelectedNodeName in
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
        .alert(isPresented: $showParseUrlAlert) {
            Alert(
                title: Text("Unable to parse URL"),
                message: Text(parseUrlMessage),
                dismissButton: .default(Text("OK")) {
                    showParseUrlAlert = false
                    parseUrlMessage = ""
                    PopupManager.dismiss()
                }
            )
        }
    }
}

#Preview {
    NodeSelectionView()
}
