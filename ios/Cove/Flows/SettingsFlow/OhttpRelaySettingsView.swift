import MijickPopups
import SwiftUI

struct OhttpRelaySettingsView: View {
    private let config = Database().globalConfig()

    @State private var input: String
    @State private var isSaving = false
    @State private var showInvalidUrlAlert = false
    @State private var showUpdateFailedAlert = false
    @FocusState private var isInputFocused: Bool

    init() {
        let config = Database().globalConfig()
        _input = State(initialValue: config.ohttpRelayUrl() ?? "")
    }

    private var defaultRelays: [String] {
        [
            "https://relay.payjoin.org",
            "https://ohttp.achow101.com",
            "https://pj.bobspacebkk.com",
        ]
    }

    var body: some View {
        Form {
            Section {
                Text(
                    "PayJoin uses an OHTTP relay to send transactions privately. By default Cove rotates between three public relays. You can specify your own relay for extra privacy."
                )
                .font(.footnote)
                .foregroundStyle(.secondary)
            } header: {
                Text("Description")
            }

            Section("Default Relays") {
                ForEach(defaultRelays, id: \.self) { relay in
                    Text(relay)
                        .font(.footnote.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            }

            Section("Custom Relay") {
                TextField("https://your-relay.example.com", text: $input)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .lineLimit(1)
                    .submitLabel(.done)
                    .focused($isInputFocused)
                    .onSubmit(save)

                Button("Save", action: save)
                    .disabled(isSaving || input == (config.ohttpRelayUrl() ?? ""))

                Button("Reset to Default", role: .destructive, action: reset)
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("PayJoin Relay")
        .alert("Invalid URL", isPresented: $showInvalidUrlAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Enter a valid HTTPS URL for the OHTTP relay.")
        }
        .alert("Unable to Update Relay", isPresented: $showUpdateFailedAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Try again later.")
        }
    }

    private func save() {
        guard !isSaving else { return }

        let inputToSave = input

        isSaving = true
        isInputFocused = false

        do {
            let normalized = try config.setOhttpRelayUrl(url: inputToSave)
            input = normalized ?? ""

            Task { @MainActor in
                await dismissAllPopups()
                try? await Task.sleep(for: .milliseconds(250))
                await MiddlePopup(state: .success("Relay saved successfully"))
                    .dismissAfter(2)
                    .present()
            }
        } catch DatabaseError.GlobalConfig(.InvalidOhttpRelayUrl) {
            showInvalidUrlAlert = true
        } catch {
            showUpdateFailedAlert = true
        }

        isSaving = false
    }

    private func reset() {
        do {
            try config.clearOhttpRelayUrl()
            input = ""
        } catch {
            showUpdateFailedAlert = true
        }
    }
}

#Preview {
    OhttpRelaySettingsView()
        .environment(AppManager.shared)
}
