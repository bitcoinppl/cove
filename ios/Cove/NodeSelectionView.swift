//
//  NodeSelectionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/18/24.
//

import PopupView
import SwiftUI

struct NodeSelectionView: View {
    // public
    @Binding var popupState: PopupState

    // private
    private let nodeSelector = NodeSelector()

    @State private var selectedNodeName: String
    private var nodeList: [NodeSelection]

    @State private var nodeIsChecking = false
    @State private var customUrl: String = ""

    @State private var showParseUrlAlert = false
    @State private var parseUrlMessage = ""

    init(popupState: Binding<PopupState>) {
        _popupState = popupState

        selectedNodeName = nodeSelector.selectedNode().name
        nodeList = nodeSelector.nodeList()
    }

    var showCustomUrlField: Bool {
        selectedNodeName.hasPrefix("Custom")
    }

    @MainActor
    private func startLoading() {
        popupState = .loading
    }

    @MainActor
    private func completeLoading(_ state: PopupState) {
        popupState = state
    }

    @ViewBuilder
    var CustomFields: some View {
        TextField("Enter \(selectedNodeName) URL", text: $customUrl)
            .textInputAutocapitalization(.never)

        Button("Save \(selectedNodeName)") {
            var node: Node? = nil
            do {
                node = try nodeSelector.parseCustomNode(url: customUrl, name: selectedNodeName)
                customUrl = node?.url ?? ""
            } catch {
                showParseUrlAlert = true
                switch error {
                case let NodeSelectorError.ParseNodeUrlError(error_string):
                    parseUrlMessage = error_string
                default:
                    parseUrlMessage = "Unknown error \(error.localizedDescription)"
                }
            }

            if let node = node {
                Task {
                    await startLoading()
                    let result = await Result { try await nodeSelector.checkAndSaveNode(node: node) }

                    switch result {
                    case .success: await completeLoading(.success("Node is healthy"))
                    case let .failure(error): await completeLoading(.failure("Failed to connect to node \(error.localizedDescription)"))
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
                return
            }

            guard let node = try? nodeSelector.selectPresetNode(name: newSelectedNodeName) else { return }

            startLoading()
            Task {
                do {
                    try await nodeSelector.checkSelectedNode(node: node)
                    completeLoading(.success("Succesfully connected to \(node.url)"))
                } catch {
                    completeLoading(.failure("Failed to connect to \(node.url), reason: \(error.localizedDescription)"))
                }
            }
        }
        .alert(isPresented: $showParseUrlAlert) {
            Alert(
                title: Text("Unable to parse URL"),
                message: Text(parseUrlMessage),
                dismissButton: .default(Text("OK")) {
                    showParseUrlAlert = false
                    parseUrlMessage = ""
                }
            )
        }
    }
}

#Preview {
    NodeSelectionView(popupState: Binding.constant(PopupState.initial))
}
