import LocalAuthentication
import SwiftUI

private enum SheetState: Equatable {
    case newPin, removePin, changePin, disableBiometric, enableAuth, enableBiometric
}

private enum AlertState: Equatable {
    case networkChanged(Network)
    case confirmEnableWipeMePin
}

struct SettingsScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(\.dismiss) private var dismiss

    @State private var notificationFrequency = 1
    @State private var networkChanged = false

    @State private var sheetState: TaggedItem<SheetState>? = nil
    @State private var alertState: TaggedItem<AlertState>? = nil

    let themes = allColorSchemes()

    private func canUseBiometrics() -> Bool {
        let context = LAContext()
        var error: NSError?
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    }

    var toggleBiometric: Binding<Bool> {
        Binding(
            get: { auth.authType == AuthType.both || auth.authType == AuthType.biometric },
            set: { enable in
                if enable {
                    sheetState = .init(.enableBiometric)
                } else {
                    sheetState = .init(.disableBiometric)
                }
            }
        )
    }

    var togglePin: Binding<Bool> {
        Binding(
            get: { auth.authType == AuthType.both || auth.authType == AuthType.pin },
            set: { enable in
                if enable { sheetState = .init(.newPin) } else { sheetState = .init(.removePin) }
            }
        )
    }

    var toggleWipeMePin: Binding<Bool> {
        Binding(
            get: { false },
            set: { enable in
                if enable { alertState = .init(.confirmEnableWipeMePin) }
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
                if canUseBiometrics() {
                    Toggle(isOn: toggleBiometric) {
                        Label("Enable Face ID", systemImage: "faceid")
                    }
                }

                Toggle(isOn: togglePin) {
                    Label("Enable PIN", systemImage: "lock")
                }

                if togglePin.wrappedValue {
                    Button(action: { sheetState = .init(.changePin) }) {
                        Label("Change PIN", systemImage: "lock.open.rotation")
                    }

                    Toggle(isOn: toggleWipeMePin) {
                        Label("Enable Wipe Data PIN", systemImage: "trash.slash")
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
                            alertState = .init(.networkChanged(app.selectedNetwork))
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
        .fullScreenCover(item: $sheetState, content: SheetContent)
        .alert(
            alertTitle,
            isPresented: showingAlert,
            presenting: alertState,
            actions: { MyAlert($0).actions },
            message: { MyAlert($0).message }
        )
        .preferredColorScheme(app.colorScheme)
        .gesture(
            networkChanged
                ? DragGesture()
                    .onChanged { gesture in
                        if gesture.startLocation.x < 25, gesture.translation.width > 100 {
                            withAnimation(.spring()) {
                                alertState = .init(.networkChanged(app.selectedNetwork))
                            }
                        }
                    }
                    .onEnded { gesture in
                        if gesture.startLocation.x < 20, gesture.translation.width > 50 {
                            withAnimation(.spring()) {
                                alertState = .init(.networkChanged(app.selectedNetwork))
                            }
                        }
                    } : nil
        )
    }

    func setPin(_ pin: String) {
        auth.dispatch(action: .setPin(pin))
        sheetState = .none
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

    // MARK: Alerts

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { alertState != nil },
            set: { if !$0 { alertState = .none } }
        )
    }

    private var alertTitle: String {
        guard let alertState else { return "Error" }
        return MyAlert(alertState).title
    }

    private func MyAlert(_ alert: TaggedItem<AlertState>) -> some AlertBuilderProtocol {
        switch alert.item {
        case let .networkChanged(network):
            return AlertBuilder(
                title: "⚠️ Network Changed ⚠️",
                message: "You've changed your network to \(network)",
                actions: {
                    Button("Yes, Change Network") {
                        app.resetRoute(to: .listWallets)
                        dismiss()
                    }
                    Button("Cancel", role: .cancel) {}
                }
            )

        case .confirmEnableWipeMePin:
            return AlertBuilder(
                title: "Are you sure?",
                message:
                    """

                    Enabling the Wipe Data PIN will let you chose a PIN that if entered will wipe all Cove wallet data on this device.

                    If you wipe the data without having a back up of your wallet, you will lose the bitcoin in that wallet. 

                    Please make sure you have a backup of your wallet before enabling this.

                    Note: Enabling the Wipe Data PIN will disable FaceID auth if its enabled.
                    """,
                actions: {
                    Button("Yes, Enable Wipe Data PIN") {
                        // app.dispatch(action: .enableWipeMePin)
                        dismiss()
                    }
                    Button("Cancel", role: .cancel) {
                        alertState = .none
                    }
                }
            )
        }
    }

    // MARK: Sheets

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .enableAuth:
            if canUseBiometrics() {
                LockView(
                    lockType: .biometric,
                    isPinCorrect: { _ in true },
                    onUnlock: { pin in
                        auth.dispatch(action: .enableBiometric)
                        if !pin.isEmpty { auth.dispatch(action: .setPin(pin)) }

                        sheetState = .none
                    },
                    backAction: { sheetState = .none },
                    content: { EmptyView() }
                )
            } else {
                NewPinView(onComplete: setPin, backAction: { sheetState = .none })
            }

        case .newPin:
            NewPinView(onComplete: setPin, backAction: { sheetState = .none })

        case .removePin:
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: auth.checkPin,
                backAction: { sheetState = .none },
                onUnlock: { _ in
                    auth.dispatch(action: .disablePin)
                    sheetState = .none
                }
            )

        case .changePin:
            ChangePinView(
                isPinCorrect: auth.checkPin,
                backAction: { sheetState = .none },
                onComplete: setPin
            )

        case .disableBiometric:
            LockView(
                lockType: auth.authType,
                isPinCorrect: auth.checkPin,
                onUnlock: { _ in
                    auth.dispatch(action: .disableBiometric)
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
                    auth.dispatch(action: .enableBiometric)
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
