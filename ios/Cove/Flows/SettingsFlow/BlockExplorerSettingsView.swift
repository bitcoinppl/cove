import MijickPopups
import SwiftUI

struct BlockExplorerSettingsView: View {
    private let config = Database().globalConfig()
    private static let previewTransactionId = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    private static let knownBitcoinTransactionId = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"

    @State private var selectedNetwork: Network
    @State private var input: String
    @State private var preview: String
    @State private var selectedOption: BlockExplorerOption
    @State private var validationError: String?
    @State private var isSaving = false
    @State private var saveTask: Task<Void, Never>?
    @State private var showInvalidUrlAlert = false
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

            Section {
                Text(
                    "Block explorers are public websites for checking Bitcoin transaction details and confirmations. Cove opens the selected explorer when you view a transaction."
                )
                .font(.footnote)
                .foregroundStyle(.secondary)
            } header: {
                Text("Description")
            }

            Section("Preview") {
                Text(preview)
                    .font(.footnote.monospaced())
                    .textSelection(.enabled)
            }

            Section("Explorer") {
                ForEach(allBlockExplorerOptions(), id: \.self) { option in
                    blockExplorerOptionRow(option)
                }
            }

            if selectedOption == .custom {
                Section("Custom") {
                    TextField("URL or template", text: $input)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .lineLimit(1)
                        .submitLabel(.done)
                        .focused($isInputFocused)
                        .onChange(of: input) { _, newValue in
                            updatePreview(for: newValue)
                        }
                        .onSubmit(save)

                    if let validationError {
                        Text(validationError)
                            .font(.caption)
                            .foregroundStyle(.red)
                    }

                    Button("Save", action: save)
                        .disabled(isSaving || input == (config.customBlockExplorer(network: selectedNetwork) ?? ""))

                    Button("Reset to Default", role: .destructive, action: reset)
                }
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Block Explorer")
        .onChange(of: selectedNetwork) { _, _ in
            reload()
        }
        .onDisappear {
            saveTask?.cancel()
        }
        .alert("Invalid URL", isPresented: $showInvalidUrlAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Enter a valid URL, IP address, or block explorer template.")
        }
    }

    private func blockExplorerOptionRow(_ option: BlockExplorerOption) -> some View {
        HStack {
            Text(option.displayName())

            Spacer()

            if selectedOption == option {
                Image(systemName: "checkmark")
                    .foregroundStyle(.blue)
                    .font(.footnote)
                    .fontWeight(.semibold)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            select(option)
        }
    }

    private func select(_ option: BlockExplorerOption) {
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
            validationError = nil
        } catch {
            validationError = error.localizedDescription
        }
    }

    private func reload() {
        input = config.customBlockExplorer(network: selectedNetwork) ?? ""
        preview = config.effectiveBlockExplorerPreview(network: selectedNetwork)
        selectedOption = config.selectedBlockExplorerOption(network: selectedNetwork)
        validationError = nil
    }

    private func updatePreview(for value: String) {
        do {
            preview = try config.previewCustomBlockExplorer(
                network: selectedNetwork,
                input: value
            )
        } catch {
            // keep save as the validation point while the user is still typing
        }

        validationError = nil
    }

    private func save() {
        guard !isSaving else { return }

        let inputToSave = input
        let networkToSave = selectedNetwork
        let checkUrl: URL

        do {
            checkUrl = try blockExplorerCheckUrl(
                network: networkToSave,
                input: inputToSave
            )
        } catch {
            validationError = error.localizedDescription
            showInvalidUrlAlert = true
            return
        }

        isSaving = true
        isInputFocused = false

        saveTask = Task { @MainActor in
            await MiddlePopup(state: .loading, message: "Checking URL").present()

            do {
                try await checkBlockExplorerUrl(checkUrl)

                let normalized = try config.setCustomBlockExplorer(
                    network: networkToSave,
                    input: inputToSave
                )
                input = normalized ?? ""
                preview = config.effectiveBlockExplorerPreview(network: networkToSave)
                selectedOption = .custom
                if normalized == nil {
                    selectedOption = config.selectedBlockExplorerOption(network: networkToSave)
                }
                validationError = nil

                await dismissAllPopups()
                try? await Task.sleep(for: .milliseconds(250))
                await MiddlePopup(state: .success("Block explorer saved successfully"))
                    .dismissAfter(2)
                    .present()
            } catch is CancellationError {
                await dismissAllPopups()
            } catch {
                let message = error.localizedDescription
                validationError = message

                await dismissAllPopups()
                try? await Task.sleep(for: .milliseconds(250))
                await MiddlePopup(state: .failure(message))
                    .dismissAfter(7)
                    .present()
            }

            isSaving = false
            saveTask = nil
        }
    }

    private func blockExplorerCheckUrl(network: Network, input: String) throws -> URL {
        let previewUrl = try config.previewCustomBlockExplorer(network: network, input: input)
        let checkUrl = previewUrl.replacingOccurrences(
            of: Self.previewTransactionId,
            with: knownTransactionId(for: network)
        )

        guard let url = URL(string: checkUrl) else {
            throw BlockExplorerCheckError.invalidUrl
        }

        return url
    }

    private func knownTransactionId(for network: Network) -> String {
        switch network {
        case .bitcoin:
            Self.knownBitcoinTransactionId
        case .testnet, .testnet4, .signet:
            Self.knownBitcoinTransactionId
        }
    }

    private func checkBlockExplorerUrl(_ url: URL) async throws {
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        request.timeoutInterval = 10

        let (_, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw BlockExplorerCheckError.invalidResponse
        }

        guard 200 ..< 300 ~= httpResponse.statusCode else {
            throw BlockExplorerCheckError.unsuccessfulStatus(httpResponse.statusCode)
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

private enum BlockExplorerCheckError: LocalizedError {
    case invalidUrl
    case invalidResponse
    case unsuccessfulStatus(Int)

    var errorDescription: String? {
        switch self {
        case .invalidUrl:
            "Invalid URL"
        case .invalidResponse:
            "Block explorer check did not return an HTTP response"
        case let .unsuccessfulStatus(statusCode):
            "Block explorer returned HTTP \(statusCode) for the test transaction"
        }
    }
}

#Preview {
    BlockExplorerSettingsView()
        .environment(AppManager.shared)
}
