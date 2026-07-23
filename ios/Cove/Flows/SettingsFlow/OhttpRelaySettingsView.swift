import MijickPopups
import SwiftUI

struct OhttpRelaySettingsView: View {
    private let config = Database().globalConfig()

    @State private var relays: [String]
    @State private var newInput: String = ""
    @State private var isAdding: Bool = false
    @State private var showInvalidUrlAlert = false
    @State private var showUpdateFailedAlert = false
    @FocusState private var isInputFocused: Bool

    init() {
        let config = Database().globalConfig()
        _relays = State(initialValue: config.ohttpRelayUrls())
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
                    "PayJoin uses an OHTTP relay to send transactions privately. By default Cove rotates between three public relays. Adding custom relays replaces the defaults."
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

            Section {
                ForEach(relays, id: \.self) { relay in
                    Text(relay)
                        .font(.footnote.monospaced())
                        .textSelection(.enabled)
                }
                .onDelete(perform: deleteRelay)

                if isAdding {
                    HStack {
                        TextField("https://your-relay.example.com", text: $newInput)
                            .focused($isInputFocused)
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .submitLabel(.done)
                            .onSubmit(addRelay)

                        Button("Add", action: addRelay)
                            .disabled(newInput.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                } else {
                    Button {
                        isAdding = true
                        isInputFocused = true
                    } label: {
                        Label("Add Relay", systemImage: "plus")
                    }
                }
            } header: {
                Text("Custom Relays")
            } footer: {
                if relays.isEmpty {
                    Text(
                        "No custom relays set. Using the three default relays, chosen randomly per send."
                    )
                    .font(.footnote)
                }
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

    private func deleteRelay(at offsets: IndexSet) {
        var updated = relays
        updated.remove(atOffsets: offsets)
        save(relays: updated, showSuccess: false)
    }

    private func addRelay() {
        let url = newInput.trimmingCharacters(in: .whitespaces)
        guard !url.isEmpty else { return }
        save(relays: relays + [url])
    }

    private func save(relays newRelays: [String], showSuccess: Bool = true) {
        isInputFocused = false

        do {
            let saved = try config.setOhttpRelayUrls(urls: newRelays)
            relays = saved
            newInput = ""
            isAdding = false

            if showSuccess {
                Task { @MainActor in
                    await dismissAllPopups()
                    try? await Task.sleep(for: .milliseconds(250))
                    await MiddlePopup(state: .success("Relay saved successfully"))
                        .dismissAfter(2)
                        .present()
                }
            }
        } catch DatabaseError.GlobalConfig(.InvalidOhttpRelayUrl) {
            showInvalidUrlAlert = true
        } catch {
            showUpdateFailedAlert = true
        }
    }
}

#Preview {
    OhttpRelaySettingsView()
        .environment(AppManager.shared)
}
