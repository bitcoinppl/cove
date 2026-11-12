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

    @State private var selectedRow: NodeRowSelection
    @State private var presetNodes: [Node]

    @State private var customNodeName: String = ""
    @State private var customUrl: String = ""

    @State private var showParseUrlAlert = false
    @State private var parseUrlMessage = ""

    @State private var checkUrlTask: Task<Void, Never>?

    @State private var lanSearch: LanSearch = .notStarted

    init() {
        let savedNode = nodeSelector.selectedNode()
        selectedRow = NodeRowSelection(savedNode)
        presetNodes = nodeSelector.presetNodes()

        // a saved custom node opens in the form for its api
        if case let .custom(node) = savedNode {
            _customUrl = State(initialValue: node.url)
            _customNodeName = State(initialValue: node.name)
        }
    }

    /// The api the custom form saves as, which only a custom row decides
    var customNodeApiType: ApiType? {
        guard case let .custom(apiType) = selectedRow else { return nil }
        return apiType
    }

    var showCustomUrlField: Bool {
        customNodeApiType != nil
    }

    /// The saved custom node's url, which a server found on the network is saved as once picked
    private var selectedCustomUrl: String? {
        guard case let .custom(node) = nodeSelector.selectedNode() else { return nil }
        return node.url
    }

    /// Whether the custom form holds something other than the saved custom node
    private var customFormHasUnsavedChanges: Bool {
        guard let apiType = customNodeApiType else { return false }
        guard case let .custom(savedNode) = nodeSelector.selectedNode() else { return true }

        return savedNode.apiType != apiType || savedNode.url != customUrl || savedNode.name != customNodeName
    }

    /// Whether a selection is still being checked, which lasts until the check itself ends:
    /// closing the loading popup cancels only the Swift task, never the Rust check and save
    private var isCheckingSelection: Bool {
        checkUrlTask != nil
    }

    func cancelCheckUrlTask() {
        if let checkUrlTask {
            checkUrlTask.cancel()
        }
    }

    @MainActor
    private func refreshNodeState() {
        let refreshedNodeSelector = NodeSelector()
        presetNodes = refreshedNodeSelector.presetNodes()
        selectedRow = NodeRowSelection(refreshedNodeSelector.selectedNode())
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
        guard let apiType = customNodeApiType else { return }
        var node: Node? = nil

        do {
            node = try nodeSelector.parseCustomNode(url: customUrl, apiType: apiType, enteredName: customNodeName)
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
            showLoadingPopup()
            checkUrlTask = Task {
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

    private func searchLocalNetwork() {
        guard lanSearch != .searching else { return }

        lanSearch = .searching

        // the search always runs to the end and records its result: the Rust search can't be
        // cancelled from Swift, and the view also disappears under the privacy cover, after which
        // a dropped result would leave the section searching forever
        Task {
            let search: LanSearch
            do {
                let nodes = try await LanNodeDiscovery.discover(nodeSelector: nodeSelector)
                search = .finished(nodes)
            } catch NodeDiscoveryError.NoLocalNetwork {
                search = .noLocalNetwork
            } catch NodeDiscoveryError.LocalNetworkBlocked {
                search = .localNetworkBlocked
            } catch {
                Log.warn("Local network node search failed: \(error.localizedDescription)")
                search = .finished([])
            }

            lanSearch = search
        }
    }

    /// Nothing reaches a discovered server until it is picked here, when the check talks to it
    private func selectDiscoveredNode(_ found: DiscoveredNode) {
        showLoadingPopup()
        checkUrlTask = Task {
            do {
                try await nodeSelector.checkAndSaveNode(node: found.node)

                // a picked server is saved as a custom node, which the custom form shows
                customUrl = found.node.url
                customNodeName = found.node.name
                refreshNodeState()
                completeLoading(.success("Connected to node successfully"))
            } catch {
                completeLoading(.failure("Failed to connect to node\n\(error.localizedDescription)"))
            }
        }
    }

    var body: some View {
        NodeSelectionForm(
            presetNodes: presetNodes,
            selectedRow: $selectedRow,
            lanSection: NodeSelectionLanSection(
                search: lanSearch,
                selectedUrl: selectedCustomUrl,
                onSearch: searchLocalNetwork,
                onSelect: selectDiscoveredNode
            ),
            customUrl: $customUrl,
            customNodeName: $customNodeName,
            saveCustomNode: checkAndSaveNode
        )
        // one selection is checked at a time, so an older check can't save over a newer choice
        .disabled(isCheckingSelection)
        .scrollContentBackground(.hidden)
        .onChange(of: selectedRow) { _, newSelectedRow in
            nodeSelectionChanged(to: newSelectedRow)
        }
        .onDisappear(perform: saveCustomFormOnLeaving)
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

    /// Saves what was typed into the custom form when the screen goes away, which includes the
    /// privacy cover when the app is backgrounded
    private func saveCustomFormOnLeaving() {
        // one selection is checked at a time: older form values must not replace a preset or a
        // found server whose check is still running
        guard !isCheckingSelection else { return }

        // an unchanged saved node is not re-checked
        if customFormHasUnsavedChanges { checkAndSaveNode() }
    }

    private func nodeSelectionChanged(to newSelectedRow: NodeRowSelection) {
        let presetName: String
        switch newSelectedRow {
        // runs even for the saved node's row, whose fields may have been cleared by the other row
        case let .custom(apiType):
            restoreCustomNodeFields(for: apiType)
            return
        case let .preset(name):
            presetName = name
        }

        guard NodeRowSelection(nodeSelector.selectedNode()) != newSelectedRow else { return }
        guard let node = try? nodeSelector.selectPresetNode(name: presetName) else { return }

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

    private func restoreCustomNodeFields(for apiType: ApiType) {
        guard case let .custom(savedSelectedNode) = nodeSelector.selectedNode() else { return }

        if savedSelectedNode.apiType == apiType {
            customUrl = savedSelectedNode.url
            customNodeName = savedSelectedNode.name
            return
        }

        // the form saves on leaving, so the saved node's own values must not be re-saved under the
        // other api, while anything typed in is kept
        if customUrl == savedSelectedNode.url, customNodeName == savedSelectedNode.name {
            customUrl = ""
            customNodeName = ""
        }
    }
}

private struct NodeSelectionForm: View {
    let presetNodes: [Node]
    @Binding var selectedRow: NodeRowSelection
    let lanSection: NodeSelectionLanSection
    @Binding var customUrl: String
    @Binding var customNodeName: String
    let saveCustomNode: () -> Void

    var body: some View {
        Form {
            NodeSelectionPresetSection(
                presetNodes: presetNodes,
                selectedRow: $selectedRow
            )
            lanSection
            NodeSelectionCustomFields(
                customRow: CustomNodeRow(selectedRow),
                customUrl: $customUrl,
                customNodeName: $customNodeName,
                save: saveCustomNode
            )
        }
    }
}

private struct NodeSelectionPresetSection: View {
    let presetNodes: [Node]
    @Binding var selectedRow: NodeRowSelection

    var body: some View {
        Section {
            ForEach(presetNodes, id: \.name) { node in
                NodeSelectionRow(
                    name: node.name,
                    isSelected: selectedRow == .preset(node.name),
                    select: { selectedRow = .preset(node.name) }
                )
            }
            ForEach(CustomNodeRow.allCases, id: \.self) { row in
                NodeSelectionRow(
                    name: row.rawValue,
                    isSelected: selectedRow == .custom(row.apiType),
                    select: { selectedRow = .custom(row.apiType) }
                )
            }
        }
    }
}

/// Where a search of the local network for nodes stands
private enum LanSearch: Equatable {
    case notStarted
    case searching
    case noLocalNetwork
    case localNetworkBlocked
    case finished([DiscoveredNode])
}

private struct NodeSelectionLanSection: View {
    let search: LanSearch
    let selectedUrl: String?
    let onSearch: () -> Void
    let onSelect: (DiscoveredNode) -> Void

    private var message: String? {
        switch search {
        case .notStarted: nil
        case .searching: "Searching this network…"
        case .noLocalNetwork: "Connect to Wi-Fi to find nodes on your network"
        case .localNetworkBlocked:
            "A VPN is blocking this network. Allow local network access in your VPN, or turn it off, to search."
        case let .finished(nodes): nodes.isEmpty ? "No Electrum servers found on this network" : nil
        }
    }

    var body: some View {
        Section {
            if case let .finished(nodes) = search {
                ForEach(nodes, id: \.node.url) { found in
                    DiscoveredNodeRow(
                        found: found,
                        isSelected: found.node.url == selectedUrl,
                        select: { onSelect(found) }
                    )
                }
            }

            if let message {
                LanSearchStatusRow(message: message, isSearching: search == .searching)
            }

            Button(search == .notStarted ? "Find Nodes on This Network" : "Search Again", action: onSearch)
                .font(.subheadline)
                .disabled(search == .searching)
        } header: {
            Text("On This Network")
        } footer: {
            if search == .finished([]) { LocalNetworkAccessHelp() }
        }
    }
}

private struct LanSearchStatusRow: View {
    let message: String
    let isSearching: Bool

    var body: some View {
        HStack(spacing: 10) {
            if isSearching { ProgressView() }
            Text(message)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
    }
}

/// Shown when a search finds nothing
///
/// iOS doesn't say whether Local Network access was granted, and can refuse the first search's
/// connections while it asks, so this covers both cases. A denied prompt never comes back, so
/// only Settings can undo it
private struct LocalNetworkAccessHelp: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("If you just allowed local network access, search again.")
            Text("If you didn't, turn on Local Network for Cove in Settings.")
            Button("Open Settings", action: openSettings)
        }
    }

    private func openSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }
}

private struct DiscoveredNodeRow: View {
    let found: DiscoveredNode
    let isSelected: Bool
    let select: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(found.host)
                    .font(.subheadline)
                Text(found.software.map { "\($0) · \(found.address)" } ?? found.address)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

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
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
    }
}

/// The row selected on the node screen
///
/// Kept apart from the saved node's name, which the person chooses, so a custom node named like a
/// preset or like a custom row still selects its own row and re-saves with its own api
private enum NodeRowSelection: Hashable {
    case preset(String)
    case custom(ApiType)

    /// A saved custom node selects the custom row for its api, which shows it in the form
    init(_ selection: NodeSelection) {
        switch selection {
        case let .preset(node): self = .preset(node.name)
        case let .custom(node): self = .custom(node.apiType)
        }
    }
}

/// The rows that open the custom node form, each saving the node with its own api
private enum CustomNodeRow: String, CaseIterable {
    case electrum = "Custom Electrum"
    case esplora = "Custom Esplora"

    init?(_ selection: NodeRowSelection) {
        switch selection {
        case .preset: return nil
        case .custom(.electrum): self = .electrum
        case .custom(.esplora): self = .esplora
        }
    }

    var apiType: ApiType {
        switch self {
        case .electrum: .electrum
        case .esplora: .esplora
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
    let customRow: CustomNodeRow?
    @Binding var customUrl: String
    @Binding var customNodeName: String
    let save: () -> Void

    var body: some View {
        if let customRow {
            Section(customRow.rawValue) {
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
