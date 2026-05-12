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
    private let db = Database()

    @Environment(AppManager.self) private var app

    @State private var selectedNodeSelection: NodeSelection
    @State private var selectedNodeName: String
    private var nodeList: [NodeSelection]

    @State private var customNodeName = ""
    @State private var customUrl = ""
    @State private var suppressCustomDraftActions = false

    @State private var showParseUrlAlert = false
    @State private var parseUrlMessage = ""

    @State private var checkUrlTask: Task<Void, Never>?

    init() {
        let selected = nodeSelector.selectedNode()
        selectedNodeSelection = selected
        selectedNodeName = selected.name
        nodeList = nodeSelector.nodeList()
    }

    private var customElectrum: String {
        "Custom Electrum"
    }

    private var customEsplora: String {
        "Custom Esplora"
    }

    var showCustomUrlField: Bool {
        if selectedNodeName == customElectrum || selectedNodeName == customEsplora {
            return true
        }
        if case .custom = selectedNodeSelection {
            return true
        }
        return false
    }

    func cancelCheckUrlTask() {
        checkUrlTask?.cancel()
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
            default:
                0
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
            Section("Custom node") {
                HStack {
                    Text("URL")
                        .frame(width: 60, alignment: .leading)

                    TextField("Enter URL", text: Binding(
                        get: { customUrl },
                        set: { value in
                            suppressCustomDraftActions = false
                            customUrl = value
                        }
                    ))
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                }
                .font(.subheadline)

                HStack {
                    Text("Name")
                        .frame(width: 60, alignment: .leading)

                    TextField("Node Name (optional)", text: Binding(
                        get: { customNodeName },
                        set: { value in
                            suppressCustomDraftActions = false
                            customNodeName = value
                        }
                    ))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                }
                .font(.subheadline)

                if suppressCustomDraftActions {
                    Text("Node already saved through Tor validation. Edit URL or name to save changes.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Button("Save Custom Node", action: checkAndSaveCustomNode)
                        .disabled(customUrl.isEmpty)
                }
            }
        }
    }

    private func customTypeName() -> String {
        if selectedNodeName == customElectrum || selectedNodeName == customEsplora {
            return selectedNodeName
        }

        if case let .custom(node) = selectedNodeSelection {
            return node.apiType == .electrum ? customElectrum : customEsplora
        }

        return selectedNodeName
    }

    private func setCustomSelection(for node: Node) {
        selectedNodeSelection = .custom(node)
        selectedNodeName = node.apiType == .electrum ? customElectrum : customEsplora
    }

    private func clearPendingNodeDraft() {
        app.clearPendingNodeTorDraft()
    }

    private func restorePendingNodeIfNeeded() {
        guard app.pendingNodeAwaitingTorSetup, !app.pendingNodeUrl.isEmpty else { return }

        customUrl = app.pendingNodeUrl
        customNodeName = app.pendingNodeName
        selectedNodeName = app.pendingNodeTypeName.isEmpty ? customElectrum : app.pendingNodeTypeName

        guard db.globalConfig().useTor(), app.pendingNodeTorValidated else { return }

        let task = Task {
            showLoadingPopup()
            do {
                let node = try nodeSelector.parseCustomNode(
                    url: app.pendingNodeUrl,
                    name: selectedNodeName,
                    enteredName: app.pendingNodeName
                )
                try await nodeSelector.checkAndSaveNode(node: node)
                setCustomSelection(for: node)
                clearPendingNodeDraft()
                suppressCustomDraftActions = true
                customUrl = ""
                customNodeName = ""
                completeLoading(.success("Connected to node successfully"))
                app.popRoute()
            } catch {
                completeLoading(.failure("Failed to connect to node\n\(error.localizedDescription)"))
            }
        }
        checkUrlTask = task
    }

    private func parseCustomNodeOrShowError() -> Node? {
        do {
            let node = try nodeSelector.parseCustomNode(
                url: customUrl,
                name: customTypeName(),
                enteredName: customNodeName
            )
            customUrl = node.url
            customNodeName = node.name
            return node
        } catch {
            showParseUrlAlert = true
            switch error {
            case let NodeSelectorError.ParseNodeUrlError(errorString):
                parseUrlMessage = errorString
            default:
                parseUrlMessage = "Unknown error \(error.localizedDescription)"
            }
            return nil
        }
    }

    func checkAndSaveCustomNode() {
        guard !customUrl.isEmpty else { return }
        guard let node = parseCustomNodeOrShowError() else { return }

        if isOnionNodeUrl(node.url) {
            if db.globalConfig().useTor() {
                let task = Task {
                    showLoadingPopup()
                    do {
                        try await nodeSelector.checkAndSaveNode(node: node)
                        setCustomSelection(for: node)
                        clearPendingNodeDraft()
                        suppressCustomDraftActions = false
                        completeLoading(.success("Connected to node successfully"))
                    } catch {
                        completeLoading(.failure("Failed to connect to node\n\(error.localizedDescription)"))
                    }
                }
                checkUrlTask = task
                return
            }

            do {
                try db.globalFlag().set(key: .torSettingsDiscovered, value: true)
                try db.globalConfig().setUseTor(useTor: true)
            } catch {
                showParseUrlAlert = true
                parseUrlMessage = "Failed to enable Tor for this onion node: \(error.localizedDescription)"
                return
            }

            setCustomSelection(for: node)
            app.pendingNodeUrl = node.url
            app.pendingNodeName = node.name
            app.pendingNodeTypeName = node.apiType == .electrum ? customElectrum : customEsplora
            app.pendingNodeAwaitingTorSetup = true
            app.pendingNodeTorValidated = false
            app.pushRoute(.settings(.network))
            return
        }

        let task = Task {
            showLoadingPopup()
            do {
                try await nodeSelector.checkAndSaveNode(node: node)
                setCustomSelection(for: node)
                completeLoading(.success("Connected to node successfully"))
            } catch {
                completeLoading(.failure("Failed to connect to node\n\(error.localizedDescription)"))
            }
        }
        checkUrlTask = task
    }

    private func selectPresetNode(_ nodeSelection: NodeSelection) {
        let node = nodeSelection.toNode()
        selectedNodeSelection = nodeSelection
        selectedNodeName = node.name
        suppressCustomDraftActions = false

        if case .custom = nodeSelection {
            customUrl = node.url
            customNodeName = node.name
            selectedNodeName = node.apiType == .electrum ? customElectrum : customEsplora
            return
        }

        customUrl = ""
        customNodeName = ""
        showLoadingPopup()
        let task = Task {
            do {
                let selected = try nodeSelector.selectPresetNode(name: node.name)
                try await nodeSelector.checkSelectedNode(node: selected)
                selectedNodeSelection = .preset(selected)
                selectedNodeName = selected.name
                completeLoading(.success("Successfully connected to \(selected.url)"))
            } catch {
                completeLoading(.failure("Failed to connect to \(node.url), reason: \(error.localizedDescription)"))
            }
        }
        checkUrlTask = task
    }

    var body: some View {
        Form {
            Section {
                ForEach(nodeList, id: \.name) { nodeSelection in
                    NodeRow(
                        nodeName: nodeSelection.name,
                        isSelected: selectedNodeSelection == nodeSelection || selectedNodeName == nodeSelection.name,
                        onTap: { selectPresetNode(nodeSelection) }
                    )
                }

                NodeRow(
                    nodeName: customElectrum,
                    isSelected: selectedNodeName == customElectrum,
                    onTap: {
                        suppressCustomDraftActions = false
                        selectedNodeName = customElectrum
                        if case let .custom(node) = selectedNodeSelection, node.apiType == .electrum {
                            customUrl = node.url
                            customNodeName = node.name
                        } else {
                            customUrl = ""
                            customNodeName = ""
                        }
                    }
                )

                NodeRow(
                    nodeName: customEsplora,
                    isSelected: selectedNodeName == customEsplora,
                    onTap: {
                        suppressCustomDraftActions = false
                        selectedNodeName = customEsplora
                        if case let .custom(node) = selectedNodeSelection, node.apiType == .esplora {
                            customUrl = node.url
                            customNodeName = node.name
                        } else {
                            customUrl = ""
                            customNodeName = ""
                        }
                    }
                )
            }

            CustomFields
        }
        .scrollContentBackground(.hidden)
        .onAppear(perform: restorePendingNodeIfNeeded)
        .onChange(of: selectedNodeName) { _, _ in
            guard showCustomUrlField, customUrl.isEmpty, !suppressCustomDraftActions else { return }
            if case let .custom(savedSelectedNode) = nodeSelector.selectedNode() {
                if savedSelectedNode.apiType == .electrum, selectedNodeName == customElectrum {
                    customUrl = savedSelectedNode.url
                    customNodeName = savedSelectedNode.name
                }

                if savedSelectedNode.apiType == .esplora, selectedNodeName == customEsplora {
                    customUrl = savedSelectedNode.url
                    customNodeName = savedSelectedNode.name
                }
            }
        }
        .onDisappear {
            cancelCheckUrlTask()
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

private struct NodeRow: View {
    let nodeName: String
    let isSelected: Bool
    let onTap: () -> Void

    var body: some View {
        HStack {
            Text(nodeName)
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
        .onTapGesture(perform: onTap)
    }
}

#Preview {
    SettingsContainer(route: .node)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
