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
            DescriptionSection()
            DefaultRelaysSection(relays: defaultRelays)
            CustomRelaysSection(
                relays: $relays,
                newInput: $newInput,
                isAdding: $isAdding,
                isInputFocused: $isInputFocused,
                onAdd: addRelay,
                onDelete: deleteRelay
            )
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

private struct DescriptionSection: View {
    var body: some View {
        Section {
            Text(
                "PayJoin uses an OHTTP relay to send transactions privately. By default Cove rotates between three public relays. Adding custom relays replaces the defaults."
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
        } header: {
            Text("Description")
        }
    }
}

private struct DefaultRelaysSection: View {
    let relays: [String]

    var body: some View {
        Section("Default Relays") {
            ForEach(relays, id: \.self) { relay in
                Text(relay)
                    .font(.footnote.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
        }
    }
}

private struct CustomRelaysSection: View {
    @Binding var relays: [String]
    @Binding var newInput: String
    @Binding var isAdding: Bool
    var isInputFocused: FocusState<Bool>.Binding
    let onAdd: () -> Void
    let onDelete: (IndexSet) -> Void

    var body: some View {
        Section {
            RelayListContent(relays: relays, onDelete: onDelete)
            AddRelayControl(
                newInput: $newInput,
                isAdding: $isAdding,
                isInputFocused: isInputFocused,
                onAdd: onAdd
            )
        } header: {
            Text("Custom Relays")
        } footer: {
            CustomRelaysFooter(isEmpty: relays.isEmpty)
        }
    }
}

private struct RelayListContent: View {
    let relays: [String]
    let onDelete: (IndexSet) -> Void

    var body: some View {
        ForEach(relays, id: \.self) { relay in
            Text(relay)
                .font(.footnote.monospaced())
                .textSelection(.enabled)
        }
        .onDelete(perform: onDelete)
    }
}

private struct AddRelayControl: View {
    @Binding var newInput: String
    @Binding var isAdding: Bool
    var isInputFocused: FocusState<Bool>.Binding
    let onAdd: () -> Void

    var body: some View {
        if isAdding {
            HStack {
                TextField("https://your-relay.example.com", text: $newInput)
                    .focused(isInputFocused)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .onSubmit(onAdd)

                Button("Add", action: onAdd)
                    .disabled(newInput.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        } else {
            Button {
                isAdding = true
                isInputFocused.wrappedValue = true
            } label: {
                Label("Add Relay", systemImage: "plus")
            }
        }
    }
}

private struct CustomRelaysFooter: View {
    let isEmpty: Bool

    var body: some View {
        if isEmpty {
            Text(
                "No custom relays set. Using the three default relays, chosen randomly per send."
            )
            .font(.footnote)
        }
    }
}

#Preview {
    OhttpRelaySettingsView()
        .environment(AppManager.shared)
}
