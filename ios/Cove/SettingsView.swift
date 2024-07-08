import SwiftUI

struct SettingsView: View {
    @Environment(MainViewModel.self) private var app
    @State private var selectedTheme = "Light"
    @State private var notificationFrequency = 1

    let themes = ["Light", "Dark", "System"]
    let notificationOptions = [1, 2, 3, 4, 5]

    var body: some View {
        Form {
            Section(header: Text("Network")) {
                Picker("Network",
                       selection: Binding(
                           get: { app.selectedNetwork },
                           set: { app.dispatch(action: .changeNetwork(network: $0)) }
                       )) {
                    ForEach(allNetworks(), id: \.self) {
                        Text($0.toString())
                    }
                }
                .pickerStyle(SegmentedPickerStyle())
            }

            Section(header: Text("Appearance")) {
                Picker("Theme", selection: $selectedTheme) {
                    ForEach(themes, id: \.self) {
                        Text($0)
                    }
                }
                .pickerStyle(SegmentedPickerStyle())
            }

            Section(header: Text("About")) {
                HStack {
                    Text("Version")
                    Spacer()
                    Text("0.0.0")
                        .foregroundColor(.secondary)
                }
            }
            .navigationTitle("Settings")
        }
    }
}
