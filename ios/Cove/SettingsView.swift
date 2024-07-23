import ActivityIndicatorView
import PopupView
import SwiftUI

struct SettingsView: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.presentationMode) private var presentationMode

    @State private var notificationFrequency = 1
    @State private var networkChanged = false
    @State private var showConfirmationAlert = false

    @State private var showPopup = false
    @State private var popUpState = PopupState.initial

    let themes = allColorSchemes()

    var popupAutoHide: Double? {
        switch popUpState {
        case .initial:
            0
        case .loading:
            nil
        case .failure(let string):
            nil
        case .success(let string):
            5
        }
    }

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
                Picker("Theme",
                       selection: Binding(
                           get: { app.colorSchemeSelection },
                           set: {
                               app.dispatch(action: .changeColorScheme($0))
                           }
                       )) {
                    ForEach(themes, id: \.self) {
                        Text($0.capitalizedString)
                    }
                }
                .pickerStyle(SegmentedPickerStyle())
            }

            NodeSelectionView(popupState: $popUpState)

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
        .popup(isPresented: $showPopup) {
            PopupMiddleView(state: popUpState)
        } customize: {
            $0.autohideIn(popupAutoHide)
                .type(.default)
                .position(.center)
                .animation(.spring())
                .closeOnTapOutside(popUpState != .loading)
                .isOpaque(true)
                .backgroundColor(.black.opacity(0.8))
        }
        .onChange(of: popUpState) { _, _ in
            if popUpState == .initial {
                showPopup = false
            } else {
                showPopup = true
            }
        }
        .navigationBarBackButtonHidden(networkChanged)
        .toolbar {
            networkChanged ?
                ToolbarItem(placement: .navigationBarLeading) {
                    Button(action: {
                        if networkChanged {
                            showConfirmationAlert = true
                        } else {
                            presentationMode.wrappedValue.dismiss()
                        }
                    }) {
                        HStack(spacing: 0) {
                            Image(systemName: "chevron.left")
                                .fontWeight(.semibold)
                                .padding(.horizontal, 0)
                            Text("Back")
                                .offset(x: 5)
                        }
                        .offset(x: -8)
                    }
                } : nil
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
        .preferredColorScheme(app.colorScheme)
        .gesture(
            networkChanged ?
                DragGesture()
                .onChanged { gesture in
                    if gesture.startLocation.x < 25, gesture.translation.width > 100 {
                        withAnimation(.spring()) {
                            showConfirmationAlert = true
                        }
                    }
                }
                .onEnded { gesture in
                    if gesture.startLocation.x < 20, gesture.translation.width > 50 {
                        withAnimation(.spring()) {
                            showConfirmationAlert = true
                        }
                    }
                } : nil
        )

        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    SettingsView()
        .environment(MainViewModel())
}
