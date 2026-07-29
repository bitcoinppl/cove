import MijickPopups
import SwiftUI

struct BlockExplorerSettingsView: View {
    private let config = Database().globalConfig()

    @State private var selectedNetwork: Network
    @State private var input: String
    @State private var preview: String
    @State private var selectedOption: BlockExplorerOption
    @State private var isSaving = false
    @State private var showInvalidUrlAlert = false
    @State private var showUpdateFailedAlert = false
    @FocusState private var isInputFocused: Bool

    init() {
        // only Bitcoin explorer overrides are editable; other networks use built-in defaults
        let network = Network.bitcoin
        let config = Database().globalConfig()
        let input = config.customBlockExplorer(network: network) ?? ""
        _selectedNetwork = State(initialValue: network)
        _input = State(initialValue: input)
        _preview = State(initialValue: config.effectiveBlockExplorerPreview(network: network))
        _selectedOption = State(initialValue: config.selectedBlockExplorerOption(network: network))
    }

    private var editableNetworks: [Network] {
        [.bitcoin]
    }

    var body: some View {
        Form {
            BlockExplorerNetworkSection(
                networks: editableNetworks,
                selectedNetwork: $selectedNetwork
            )

            BlockExplorerInformationSections(preview: preview)
            BlockExplorerOptionsSection(selectedOption: selectedOption, onSelect: select)

            if selectedOption == .custom {
                CustomBlockExplorerSection(
                    input: $input,
                    isInputFocused: $isInputFocused,
                    isSaveDisabled: isSaving
                        || input == (config.customBlockExplorer(network: selectedNetwork) ?? ""),
                    onInputChanged: updatePreview,
                    onSave: save,
                    onReset: reset
                )
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Block Explorer")
        .onChange(of: selectedNetwork) { _, _ in
            reload()
        }
        .alert("Invalid URL", isPresented: $showInvalidUrlAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Enter a valid URL, IP address, or block explorer template.")
        }
        .alert("Unable to Update Block Explorer", isPresented: $showUpdateFailedAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Try again later.")
        }
    }

    private func select(_ option: BlockExplorerOption) {
        if selectedOption == option {
            return
        }

        switch option {
        case .custom:
            selectedOption = .custom
            input = ""
            updatePreview(for: input)
        default:
            savePreset(option)
        }
    }

    private func savePreset(_ option: BlockExplorerOption) {
        do {
            let normalized = try config.setBlockExplorerOption(
                network: selectedNetwork,
                option: option
            )
            input = normalized ?? ""
            preview = config.effectiveBlockExplorerPreview(network: selectedNetwork)
            selectedOption = config.selectedBlockExplorerOption(network: selectedNetwork)
        } catch {
            showUpdateFailedAlert = true
        }
    }

    private func reload() {
        input = config.customBlockExplorer(network: selectedNetwork) ?? ""
        preview = config.effectiveBlockExplorerPreview(network: selectedNetwork)
        selectedOption = config.selectedBlockExplorerOption(network: selectedNetwork)
    }

    private func updatePreview(for value: String) {
        do {
            preview = try config.previewCustomBlockExplorer(
                network: selectedNetwork,
                input: value
            )
        } catch {
            preview = ""
        }
    }

    private func save() {
        guard !isSaving else { return }

        let inputToSave = input
        let networkToSave = selectedNetwork

        isSaving = true
        isInputFocused = false

        do {
            let normalized = try config.setCustomBlockExplorer(
                network: networkToSave,
                input: inputToSave
            )
            input = normalized ?? ""
            preview = config.effectiveBlockExplorerPreview(network: networkToSave)
            selectedOption = config.selectedBlockExplorerOption(network: networkToSave)

            Task { @MainActor in
                await dismissAllPopups()
                try? await Task.sleep(for: .milliseconds(250))
                await MiddlePopup(state: .success("Block explorer saved successfully"))
                    .dismissAfter(2)
                    .present()
            }
        } catch {
            showInvalidUrlAlert = true
        }

        isSaving = false
    }

    private func reset() {
        do {
            try config.clearCustomBlockExplorer(network: selectedNetwork)
            reload()
        } catch {
            showUpdateFailedAlert = true
        }
    }
}

private struct BlockExplorerInformationSections: View {
    let preview: String

    var body: some View {
        BlockExplorerDescriptionSection()
        BlockExplorerPreviewSection(preview: preview)
    }
}

private struct BlockExplorerNetworkSection: View {
    let networks: [Network]
    @Binding var selectedNetwork: Network

    var body: some View {
        if networks.count > 1 {
            Section {
                Picker("Network", selection: $selectedNetwork) {
                    ForEach(networks, id: \.self) { network in
                        Text(network.displayName()).tag(network)
                    }
                }
                .pickerStyle(.segmented)
            }
        }
    }
}

private struct BlockExplorerDescriptionSection: View {
    var body: some View {
        Section {
            Text(
                "Block explorers are public websites for checking Bitcoin transaction details and confirmations. Cove opens the selected explorer when you view a transaction."
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
        } header: {
            Text("Description")
        }
    }
}

private struct BlockExplorerPreviewSection: View {
    let preview: String

    var body: some View {
        Section("Preview") {
            Text(preview)
                .font(.footnote.monospaced())
                .textSelection(.enabled)
        }
    }
}

private struct BlockExplorerOptionsSection: View {
    let selectedOption: BlockExplorerOption
    let onSelect: (BlockExplorerOption) -> Void

    var body: some View {
        Section("Explorer") {
            ForEach(allBlockExplorerOptions(), id: \.self) { option in
                BlockExplorerOptionRow(
                    option: option,
                    isSelected: selectedOption == option,
                    onSelect: onSelect
                )
            }
        }
    }
}

private struct BlockExplorerOptionRow: View {
    let option: BlockExplorerOption
    let isSelected: Bool
    let onSelect: (BlockExplorerOption) -> Void

    var body: some View {
        HStack {
            Text(option.displayName())

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

    private func select() {
        onSelect(option)
    }
}

private struct CustomBlockExplorerSection: View {
    @Binding var input: String
    let isInputFocused: FocusState<Bool>.Binding
    let isSaveDisabled: Bool
    let onInputChanged: (String) -> Void
    let onSave: () -> Void
    let onReset: () -> Void

    var body: some View {
        Section("Custom") {
            TextField("URL or template", text: $input)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(1)
                .submitLabel(.done)
                .focused(isInputFocused)
                .onChange(of: input, inputChanged)
                .onSubmit(onSave)

            Button("Save", action: onSave)
                .disabled(isSaveDisabled)

            Button("Reset to Default", role: .destructive, action: onReset)
        }
    }

    private func inputChanged(_: String, _ newValue: String) {
        onInputChanged(newValue)
    }
}

#Preview {
    BlockExplorerSettingsView()
        .environment(AppManager.shared)
}
