import SwiftUI

struct BlockExplorerSettingsView: View {
    private let config = Database().globalConfig()

    @State private var selectedNetwork: Network
    @State private var input: String
    @State private var preview: String
    @State private var validationError: String?

    init() {
        // only Bitcoin explorer overrides are editable; other networks use built-in defaults
        let network = Network.bitcoin
        let config = Database().globalConfig()
        _selectedNetwork = State(initialValue: network)
        _input = State(initialValue: config.customBlockExplorer(network: network) ?? "")
        _preview = State(initialValue: config.effectiveBlockExplorerPreview(network: network))
    }

    private var editableNetworks: [Network] {
        [.bitcoin]
    }

    var body: some View {
        Form {
            if editableNetworks.count > 1 {
                Section {
                    Picker("Network", selection: $selectedNetwork) {
                        ForEach(editableNetworks, id: \.self) { network in
                            Text(network.displayName()).tag(network)
                        }
                    }
                    .pickerStyle(.segmented)
                }
            }

            Section("Preview") {
                Text(preview)
                    .font(.footnote.monospaced())
                    .textSelection(.enabled)
            }

            Section {
                TextField("URL or template", text: $input, axis: .vertical)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(2 ... 5)
                    .onChange(of: input) { _, newValue in
                        updatePreview(for: newValue)
                    }

                if let validationError {
                    Text(validationError)
                        .font(.caption)
                        .foregroundStyle(.red)
                }

                Button("Save", action: save)
                    .disabled(input == (config.customBlockExplorer(network: selectedNetwork) ?? ""))

                Button("Reset to Default", role: .destructive, action: reset)
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Block Explorer")
        .onChange(of: selectedNetwork) { _, _ in
            reload()
        }
    }

    private func reload() {
        input = config.customBlockExplorer(network: selectedNetwork) ?? ""
        preview = config.effectiveBlockExplorerPreview(network: selectedNetwork)
        validationError = nil
    }

    private func updatePreview(for value: String) {
        do {
            preview = try config.previewCustomBlockExplorer(
                network: selectedNetwork,
                input: value
            )
            validationError = nil
        } catch {
            validationError = error.localizedDescription
        }
    }

    private func save() {
        do {
            let normalized = try config.setCustomBlockExplorer(
                network: selectedNetwork,
                input: input
            )
            input = normalized ?? ""
            preview = config.effectiveBlockExplorerPreview(network: selectedNetwork)
            validationError = nil
        } catch {
            validationError = error.localizedDescription
        }
    }

    private func reset() {
        do {
            try config.clearCustomBlockExplorer(network: selectedNetwork)
            reload()
        } catch {
            validationError = error.localizedDescription
        }
    }
}

#Preview {
    BlockExplorerSettingsView()
        .environment(AppManager.shared)
}
