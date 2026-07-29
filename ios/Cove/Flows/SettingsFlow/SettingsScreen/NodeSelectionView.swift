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

    func checkAndSaveNode() {
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
                case .success:
                    refreshNodeState()
                    completeLoading(.success("Connected to node successfully"))
                case let .failure(error):
                    let errorMessage = "Failed to connect to node\n \(error.localizedDescription)"
                    let formattedMessage = errorMessage.replacingOccurrences(of: "\\n", with: "\n")

                    completeLoading(.failure(formattedMessage))
                }
            }
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
