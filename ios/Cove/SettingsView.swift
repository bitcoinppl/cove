import SwiftUI

struct SettingsView: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.presentationMode) var presentationMode

    @State private var selectedTheme = "Light"
    @State private var notificationFrequency = 1
    @State private var networkChanged = false
    @State private var showConfirmationAlert = false

    let themes = ["Light", "Dark", "System"]
    let notificationOptions = [1, 2, 3, 4, 5]

    var body: some View {
        Form {
            Section(header: Text("Network")) {
                Picker("Network",
                       selection: Binding(
                           get: { app.selectedNetwork },
                           set: {
                               networkChanged.toggle()
                               app.dispatch(action: .changeNetwork(network: $0))
                           }
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
        .navigationBarBackButtonHidden(true)
        .toolbar {
            ToolbarItem(placement: .navigationBarLeading) {
                Button(action: {
                    if networkChanged {
                        showConfirmationAlert = true
                    } else {
                        presentationMode.wrappedValue.dismiss()
                    }
                }) {
                    HStack {
                        Image(systemName: "chevron.left")
                        Text("Back")
                    }
                }
            }
        }
        .alert(isPresented: $showConfirmationAlert) {
            Alert(
                title: Text("⚠️ Network Changed ⚠️"),
                message: Text("You've changed your network to \(app.selectedNetwork)"),
                primaryButton: .destructive(Text("Yes, Change Network")) {
                    app.resetRoute(to: .listWallets)
                    presentationMode.wrappedValue.dismiss()
                },
                secondaryButton: .cancel(Text("Cancel"))
            )
        }
    }
}
