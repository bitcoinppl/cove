import LocalAuthentication
import SwiftUI

private enum SheetState: Equatable {
    case newPin, removePin, changePin, disableAuth, disableBiometric, enableAuth, enableBiometric
}

struct SettingsScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss

    @State private var notificationFrequency = 1
    @State private var networkChanged = false
    @State private var showConfirmationAlert = false

    @State private var sheetState: TaggedItem<SheetState>? = nil

    let themes = allColorSchemes()

    private func canUseBiometrics() -> Bool {
        let context = LAContext()
        var error: NSError?
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    }

    var useAuth: Binding<Bool> {
        Binding(
            get: { app.isAuthEnabled },
            set: { enable in
                if enable { return sheetState = .init(.enableAuth) }

                switch app.authType {
                case .both, .pin: sheetState = .init(.removePin)
                case .biometric: sheetState = .init(.disableAuth)
                case .none: Log.error("Trying to disable auth when auth is not enabled")
                }
            }
        )
    }

    var useBiometric: Binding<Bool> {
        Binding(
            get: { app.authType == AuthType.both || app.authType == AuthType.biometric },
            set: { enable in
                if enable { sheetState = .init(.enableBiometric) }
                else { sheetState = .init(.disableBiometric) }
            }
        )
    }

    var usePin: Binding<Bool> {
        Binding(
            get: { app.authType == AuthType.both || app.authType == AuthType.pin },
            set: { enable in
                if enable { sheetState = .init(.newPin) }
                else { sheetState = .init(.removePin) }
            }
        )
    }

    var body: some View {
        Form {
            Section(header: Text("About")) {
                HStack {
                    Text("Version")
                    Spacer()
                    Text("0.0.0")
                        .foregroundColor(.secondary)
                }
            }

            Section(header: Text("Network")) {
                Picker(
                    "Network",
                    selection: Binding(
                        get: { app.selectedNetwork },
                        set: {
                            networkChanged.toggle()
                            app.dispatch(action: .changeNetwork(network: $0))
                        }
                    )
                ) {
                    ForEach(allNetworks(), id: \.self) {
                        Text($0.toString())
                    }
                }
                .pickerStyle(SegmentedPickerStyle())
            }

            Section(header: Text("Appearance")) {
                Picker(
                    "Theme",
                    selection: Binding(
                        get: { app.colorSchemeSelection },
                        set: {
                            app.dispatch(action: .changeColorScheme($0))
                        }
                    )
                ) {
                    ForEach(themes, id: \.self) {
                        Text($0.capitalizedString)
                    }
                }
                .pickerStyle(SegmentedPickerStyle())
            }

            NodeSelectionView()

            Section("Security") {
                Toggle(isOn: useAuth) {
                    Label("Require Authentication", systemImage: "lock.shield")
                }

                if app.isAuthEnabled {
                    if canUseBiometrics() {
                        Toggle(isOn: useBiometric) {
                            Label("Enable Face ID", systemImage: "faceid")
                        }
                    }

                    Toggle(isOn: usePin) {
                        Label("Enable PIN", systemImage: "lock.fill")
                    }

                    if usePin.wrappedValue {
                        Button(action: { sheetState = .init(.changePin) }) {
                            Label("Change PIN", systemImage: "lock.open.rotation")
                        }
                    }
                }
            }
        }
        .navigationTitle("Settings")
        .navigationBarBackButtonHidden(networkChanged)
        .toolbar {
            networkChanged
                ? ToolbarItem(placement: .navigationBarLeading) {
                    Button(action: {
                        if networkChanged {
                            showConfirmationAlert = true
                        } else {
                            dismiss()
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
                    dismiss()
                },
                secondaryButton: .cancel(Text("Cancel"))
            )
        }
        .fullScreenCover(item: $sheetState, content: SheetContent)
        .preferredColorScheme(app.colorScheme)
        .gesture(
            networkChanged
                ? DragGesture()
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
    }

    func setPin(_ pin: String) {
        app.dispatch(action: .setPin(pin))
        sheetState = .none
    }

    func checkPin(_ pin: String) -> Bool {
        AuthPin().check(pin: pin)
    }

    @ViewBuilder
    private func CancelView(_ content: () -> some View) -> some View {
        VStack {
            HStack {
                Spacer()

                Button("Cancel") {
                    sheetState = .none
                }
                .foregroundStyle(.white)
                .font(.headline)
            }
            .padding()

            content()
        }
        .background(.midnightBlue)
    }

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .enableAuth:
            LockView(
                lockType: .both,
                isPinCorrect: { _ in true },
                onUnlock: { pin in
                    app.dispatch(action: .enableBiometric)

                    if !pin.isEmpty {
                        app.dispatch(action: .setPin(pin))
                    }

                    sheetState = .none
                },
                backAction: { sheetState = .none },
                content: { EmptyView() }
            )

        case .newPin:
            NewPinView(onComplete: setPin, backAction: { sheetState = .none })

        case .removePin:
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: checkPin,
                backAction: { sheetState = .none },
                onUnlock: { _ in
                    app.dispatch(action: .disablePin)
                    sheetState = .none
                }
            )

        case .changePin:
            ChangePinView(
                isPinCorrect: checkPin,
                backAction: { sheetState = .none },
                onComplete: setPin
            )

        case .disableAuth:
            LockView(
                lockType: app.authType,
                isPinCorrect: checkPin,
                onUnlock: { _ in
                    app.dispatch(action: .disableAuth)
                    sheetState = .none
                },
                backAction: { sheetState = .none },
                content: { EmptyView() }
            )

        case .disableBiometric:
            LockView(
                lockType: app.authType,
                isPinCorrect: checkPin,
                onUnlock: { _ in
                    app.dispatch(action: .disableBiometric)
                    sheetState = .none
                },
                backAction: { sheetState = .none },
                content: { EmptyView() }
            )

        case .enableBiometric:
            LockView(
                lockType: .biometric,
                isPinCorrect: { _ in true },
                onUnlock: { _ in
                    app.dispatch(action: .enableBiometric)
                    sheetState = .none
                },
                backAction: { sheetState = .none },
                content: { EmptyView() }
            )
        }
    }
}

#Preview {
    SettingsScreen()
        .environment(AppManager())
}
