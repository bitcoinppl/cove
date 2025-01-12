import LocalAuthentication
import SwiftUI

private enum SheetState: Equatable {
    case newPin,
         removePin,
         changePin,
         disableBiometric,
         enableAuth,
         enableBiometric,
         enableWipeDataPin
}

private enum AlertState: Equatable {
    case networkChanged(Network)
    case unverifiedWallets(WalletId)
    case confirmEnableWipeMePin
    case noteNoFaceIdWhenWipeMePin
    case notePinRequired
    case noteFaceIdDisabling
    case wipeDataSetPinError(String)
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
            get: { auth.type == AuthType.both || auth.type == AuthType.biometric },
            set: { enable in
                // disable
                if !enable {
                    sheetState = .init(.disableBiometric)
                    return
                }

                // enable
                if auth.isWipeDataPinEnabled {
                    alertState = .init(.noteNoFaceIdWhenWipeMePin)
                } else {
                    sheetState = .init(.enableBiometric)
                }
            }
        )
    }

    var togglePin: Binding<Bool> {
        Binding(
            get: { auth.type == AuthType.both || auth.type == AuthType.pin },
            set: { enable in
                if enable { sheetState = .init(.newPin) } else { sheetState = .init(.removePin) }
            }
        )
    }

    var toggleWipeMePin: Binding<Bool> {
        Binding(
            get: { auth.isWipeDataPinEnabled },
            set: { enable in
                // enable
                if enable {
                    if !app.rust.unverifiedWalletIds().isEmpty {
                        alertState = .init(
                            .unverifiedWallets(app.rust.unverifiedWalletIds().first!))
                        return
                    }

                    if auth.type == .biometric {
                        alertState = .init(.notePinRequired)
                        return
                    }

                    if auth.type == .both {
                        alertState = .init(.noteFaceIdDisabling)
                        return
                    }

                    alertState = .init(.confirmEnableWipeMePin)
                }

                // disable
                if !enable { auth.dispatch(action: .disableWipeDataPin) }
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
        .navigationBarTitleDisplayMode(.inline)
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

    private func MyAlert(_ alert: TaggedItem<AlertState>) -> AnyAlertBuilder {
        switch alert.item {
        case let .networkChanged(network):
            AlertBuilder(
                title: "⚠️ Network Changed ⚠️",
                message: "You've changed your network to \(network)",
                actions: {
                    Button("Yes, Change Network") {
                        app.resetRoute(to: .listWallets)
                        dismiss()
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case let .unverifiedWallets(walletId):
            AlertBuilder(
                title: "Can't Enable Wipe Data PIN",
                message: """
                You have wallets that have not been backed up. Please back up your wallets before enabling the Wipe Data PIN.\
                If you wipe the data without having a back up of your wallet, you will lose the bitcoin in that wallet.
                """,
                actions: {
                    Button("Go To Wallet") {
                        try? app.rust.selectWallet(id: walletId)
                    }

                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case .confirmEnableWipeMePin:
            AlertBuilder(
                title: "Are you sure?",
                message:
                """

                Enabling the Wipe Data PIN will let you chose a PIN that if entered will wipe all Cove wallet data on this device.

                If you wipe the data without having a back up of your wallet, you will lose the bitcoin in that wallet. 

                Please make sure you have a backup of your wallet before enabling this.
                """,
                actions: {
                    Button("Yes, Enable Wipe Data PIN") {
                        alertState = .none
                        sheetState = .init(.enableWipeDataPin)
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case .notePinRequired:
            AlertBuilder(
                title: "PIN is required",
                message: "Setting a PIN is required to have a wipe data PIN",
                actions: { Button("OK") { alertState = .none } }
            ).eraseToAny()

        case .noteFaceIdDisabling:
            AlertBuilder(
                title: "Disable FaceID Unlock?",
                message: """

                Enabling the wipe data PIN will disable FaceID unlock for Cove. 

                Going forward, you will have to use your PIN to unlock Cove.
                """,
                actions: {
                    Button("Disable FaceID", role: .destructive) {
                        auth.dispatch(action: .disableBiometric)
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.350) {
                            alertState = .init(.confirmEnableWipeMePin)
                        }
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case .noteNoFaceIdWhenWipeMePin:
            AlertBuilder(
                title: "Can't do that",
                message: "You can't have both Wipe Data PIN and FaceID active at the same time",
                actions: {
                    Button("Cancel", role: .cancel) { alertState = .none }
                    Button("Disable Wipe Data PIN", role: .destructive) {
                        auth.dispatch(action: .disableWipeDataPin)
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.350) {
                            auth.dispatch(action: .enableBiometric)
                        }
                    }
                }
            ).eraseToAny()

        case let .wipeDataSetPinError(error):
            AlertBuilder(
                title: "Something went wrong!",
                message: error,
                actions: { Button("OK") { alertState = .none } }
            )
            .eraseToAny()
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
                    auth.dispatch(action: .disableWipeDataPin)
                    sheetState = .none
                }
            )

        case .changePin:
            ChangePinView(
                isPinCorrect: auth.checkPin,
                backAction: { sheetState = .none },
                onComplete: { pin in
                    sheetState = .none

                    if auth.checkWipeDataPin(pin) {
                        alertState = .init(
                            .wipeDataSetPinError(
                                "Can't update PIN because its the same as your wipe data PIN")
                        )
                        return
                    }

                    setPin(pin)
                }
            )

        case .disableBiometric:
            LockView(
                lockType: auth.type,
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

        case .enableWipeDataPin:
            WipeDataPinView(
                onComplete: setWipeDataPin,
                backAction: {
                    sheetState = .none
                }
            )
        }
    }

    func setWipeDataPin(_ pin: String) {
        sheetState = .none

        do { try auth.rust.setWipeDataPin(pin: pin) } catch {
            let error = error as! AuthManagerError
            alertState = .init(.wipeDataSetPinError(error.describe))
        }
    }
}

#Preview {
    SettingsScreen()
        .environment(AppManager())
}
