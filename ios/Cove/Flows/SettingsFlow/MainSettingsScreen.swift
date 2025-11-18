import LocalAuthentication
import SwiftUI

private enum SheetState: Equatable {
    case newPin
    case removePin
    case removeAllTrickPins
    indirect case removeWipeDataPin(TaggedItem<SheetState>? = .none)
    indirect case removeDecoyPin(TaggedItem<SheetState>? = .none)
    case changePin
    case disableBiometric
    case enableAuth
    case enableBiometric
    case enableWipeDataPin
    case enableDecoyPin
}

private enum AlertState: Equatable {
    case networkChanged(Network)
    case unverifiedWallets(WalletId)
    case confirmEnableWipeMePin
    case confirmDecoyPin
    case noteNoFaceIdWhenTrickPins
    case noteNoFaceIdWhenWipeMePin
    case noteNoFaceIdWhenDecoyPin
    case notePinRequired
    indirect case noteFaceIdDisabling(AlertState)
    case extraSetPinError(String)
}

struct MainSettingsScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme

    // private
    @State private var sheetState: TaggedItem<SheetState>? = nil
    @State private var alertState: TaggedItem<AlertState>? = nil

    // settings toggles for when you are in decoy mode
    @State private var isPinEnabled: Bool = true
    @State private var isDecoyPinEnabled: Bool = false
    @State private var isFaceIdEnabled: Bool = false
    @State private var isWipeDataPinEnabled: Bool = false

    let themes = allColorSchemes()

    var networkChanged: Bool {
        if app.previousSelectedNetwork == nil { return false }
        return app.selectedNetwork != app.previousSelectedNetwork
    }

    private func canUseBiometrics() -> Bool {
        let context = LAContext()
        var error: NSError?
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    }

    // MARK: Binding toggles

    var toggleBiometric: Binding<Bool> {
        Binding(
            get: {
                if auth.isInDecoyMode() { return isFaceIdEnabled }
                return auth.type == AuthType.both || auth.type == AuthType.biometric
            },
            set: { enable in
                if auth.isInDecoyMode() { return isFaceIdEnabled = enable }

                // disable
                if !enable {
                    sheetState = .init(.disableBiometric)
                    return
                }

                // enable
                if auth.isDecoyPinEnabled, auth.isWipeDataPinEnabled {
                    alertState = .init(.noteNoFaceIdWhenTrickPins)
                    return
                }

                if auth.isWipeDataPinEnabled {
                    alertState = .init(.noteNoFaceIdWhenWipeMePin)
                    return
                }

                if auth.isDecoyPinEnabled {
                    alertState = .init(.noteNoFaceIdWhenDecoyPin)
                    return
                }

                sheetState = .init(.enableBiometric)
            }
        )
    }

    var togglePin: Binding<Bool> {
        Binding(
            get: {
                if auth.isInDecoyMode() { return isPinEnabled }
                return auth.type == AuthType.both || auth.type == AuthType.pin
            },
            set: { enable in
                if enable { sheetState = .init(.newPin) } else { sheetState = .init(.removePin) }
            }
        )
    }

    var toggleWipeMePin: Binding<Bool> {
        Binding(
            get: {
                if auth.isInDecoyMode() { return isWipeDataPinEnabled }
                return auth.isWipeDataPinEnabled
            },
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
                        alertState = .init(.noteFaceIdDisabling(.confirmEnableWipeMePin))
                        return
                    }

                    alertState = .init(.confirmEnableWipeMePin)
                }

                // disable
                if !enable { sheetState = .init(.removeWipeDataPin()) }
            }
        )
    }

    var toggleDecoyPin: Binding<Bool> {
        Binding(
            get: {
                if auth.isInDecoyMode() { return isDecoyPinEnabled }
                return auth.isDecoyPinEnabled
            },
            set: { enable in
                // pretend to turn it off if you are in decoy mode
                if !enable, auth.isInDecoyMode() { return isDecoyPinEnabled = false }

                // enable
                if enable {
                    if auth.type == .biometric {
                        alertState = .init(.notePinRequired)
                        return
                    }

                    if auth.type == .both {
                        alertState = .init(.noteFaceIdDisabling(.confirmDecoyPin))
                        return
                    }

                    alertState = .init(.confirmDecoyPin)
                }

                // disable
                if !enable { sheetState = .init(.removeDecoyPin()) }
            }
        )
    }

    @ViewBuilder
    var GeneralSection: some View {
        Section(header: Text("General")) {
            SettingsRow(title: "Network", route: .network, symbol: "network")
            SettingsRow(title: "Appearance", route: .appearance, symbol: "sun.max.fill")
            SettingsRow(
                title: "Node", route: .node, symbol: "point.3.filled.connected.trianglepath.dotted"
            )
            SettingsRow(title: "Currency", route: .fiatCurrency, symbol: "dollarsign.circle")
        }
    }

    var SecuritySection: some View {
        Section("Security") {
            if canUseBiometrics() {
                SettingsToggle(title: "Enable FaceID", symbol: "faceid", item: toggleBiometric)
            }

            SettingsToggle(title: "Enable PIN", symbol: "lock", item: togglePin)

            if togglePin.wrappedValue {
                SettingsRow(title: "Change PIN", symbol: "lock.open.rotation") {
                    sheetState = .init(.changePin)
                }
                .foregroundStyle(.blue)

                SettingsToggle(
                    title: "Enable Wipe Data PIN",
                    symbol: "exclamationmark.lock.fill",
                    item: toggleWipeMePin
                )

                SettingsToggle(
                    title: "Enable Decoy PIN",
                    symbol: "theatermasks",
                    item: toggleDecoyPin
                )
            }
        }
    }

    var body: some View {
        ZStack(alignment: .bottom) {
            VStack {
                HStack { Text(app.rust.debugOrRelease()) }
                HStack { Text(app.fullVersionId) }
                HStack { Text("feedback@covebitcoinwallet.com") }
            }
            .foregroundStyle(Color(UIColor.tertiaryLabel))
            .font(.caption2)
            .tint(Color(UIColor.tertiaryLabel))

            Form {
                GeneralSection
                WalletSettingsSection()
                SecuritySection
            }
        }
        .scrollContentBackground(.hidden)
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
                        app.confirmNetworkChange()
                        app.rust.selectLatestOrNewWallet()
                        dismiss()
                    }
                    Button("Cancel", role: .cancel) {
                        alertState = .none
                    }
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

        case .confirmDecoyPin:
            AlertBuilder(
                title: "Are you sure?",
                message:
                """

                Enabling Decoy PIN will let you chose a PIN that if entered, will show you a different set of wallets.

                These wallets will only be accessible by entering the decoy PIN instead of your regular PIN.

                To access your regular wallets, you will have to close the app, start it again and enter your regular PIN.
                """,
                actions: {
                    Button("Yes, Enable Decoy PIN") {
                        alertState = .none
                        sheetState = .init(.enableDecoyPin)
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

        case let .noteFaceIdDisabling(nextAlertState):
            AlertBuilder(
                title: "Disable FaceID Unlock?",
                message: """

                Enabling this trick PIN will disable FaceID unlock for Cove. 

                Going forward, you will have to use your PIN to unlock Cove.
                """,
                actions: {
                    Button("Disable FaceID", role: .destructive) {
                        auth.dispatch(action: .disableBiometric)
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.350) {
                            alertState = .init(nextAlertState)
                        }
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case .noteNoFaceIdWhenTrickPins:
            AlertBuilder(
                title: "Can't do that",
                message: """

                You can't have Decoy PIN & Wipe Data Pin enabled and FaceID active at the same time.

                Do you wan't to disable both of these trick PINs and enable FaceID?
                """,
                actions: {
                    Button("Cancel", role: .cancel) { alertState = .none }
                    Button("Yes, Disable trick PINs", role: .destructive) {
                        sheetState = .init(.removeAllTrickPins)
                    }
                }
            ).eraseToAny()

        case .noteNoFaceIdWhenWipeMePin:
            AlertBuilder(
                title: "Can't do that",
                message: "You can't have both Wipe Data PIN and FaceID active at the same time",
                actions: {
                    Button("Cancel", role: .cancel) { alertState = .none }
                    Button("Disable Wipe Data PIN", role: .destructive) {
                        if !auth.isDecoyPinEnabled {
                            let nextSheetState = TaggedItem(SheetState.enableBiometric)
                            sheetState = .init(.removeWipeDataPin(nextSheetState))
                        } else {
                            sheetState = .init(.removeWipeDataPin(.none))
                        }
                    }
                }
            ).eraseToAny()

        case .noteNoFaceIdWhenDecoyPin:
            AlertBuilder(
                title: "Can't do that",
                message: "You can't have both Decoy PIN and FaceID active at the same time",
                actions: {
                    Button("Cancel", role: .cancel) { alertState = .none }
                    Button("Disable Decoy Pin", role: .destructive) {
                        if !auth.isWipeDataPinEnabled {
                            let nextSheetState = TaggedItem(SheetState.enableBiometric)
                            sheetState = .init(.removeDecoyPin(nextSheetState))
                        } else {
                            sheetState = .init(.removeDecoyPin(.none))
                        }
                    }
                }
            ).eraseToAny()

        case let .extraSetPinError(error):
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
                        if auth.isInDecoyMode() { return }
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
                isPinCorrect: { pin in
                    if auth.isInDecoyMode() { return auth.checkDecoyPin(pin) }
                    return auth.checkPin(pin)
                },

                showPin: false,
                backAction: { sheetState = .none },
                onUnlock: { _ in
                    if auth.isInDecoyMode() {
                        sheetState = .none
                        return isPinEnabled = false
                    }

                    auth.dispatch(action: .disablePin)
                    auth.dispatch(action: .disableWipeDataPin)
                    sheetState = .none
                }
            )

        case let .removeWipeDataPin(nextSheet):
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: auth.checkPin,
                showPin: false,
                backAction: { sheetState = .none },
                onUnlock: { _ in
                    if auth.isInDecoyMode() { return }
                    auth.dispatch(action: .disableWipeDataPin)
                    sheetState = nextSheet
                }
            )

        case let .removeDecoyPin(nextState):
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: auth.checkPin,
                showPin: false,
                backAction: { sheetState = .none },
                onUnlock: { _ in
                    auth.dispatch(action: .disableDecoyPin)
                    sheetState = nextState
                }
            )

        case .removeAllTrickPins:
            NumberPadPinView(
                title: "Enter Current PIN",
                isPinCorrect: auth.checkPin,
                showPin: false,
                backAction: { sheetState = .none },
                onUnlock: { _ in
                    auth.dispatch(action: .disableDecoyPin)
                    auth.dispatch(action: .disableWipeDataPin)
                    sheetState = .init(.enableBiometric)
                }
            )

        case .changePin:
            ChangePinView(
                isPinCorrect: { pin in
                    if auth.isInDecoyMode() { return auth.checkDecoyPin(pin) }
                    return auth.checkPin(pin)
                },
                backAction: { sheetState = .none },
                onComplete: { pin in
                    if auth.isInDecoyMode() { return sheetState = .none }

                    sheetState = .none
                    if auth.checkWipeDataPin(pin) {
                        alertState = .init(
                            .extraSetPinError(
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

        case .enableDecoyPin:
            DecoyPinView(
                onComplete: setDecoyPin,
                backAction: {
                    sheetState = .none
                }
            )
        }
    }

    // MARK: Setter functions

    func setPin(_ pin: String) {
        if auth.isInDecoyMode() { return isPinEnabled = true }
        auth.dispatch(action: .setPin(pin))
        sheetState = .none
    }

    func setWipeDataPin(_ pin: String) {
        sheetState = .none
        if auth.isInDecoyMode() { return isWipeDataPinEnabled = true }

        do { try auth.rust.setWipeDataPin(pin: pin) } catch {
            let error = error as! AuthManagerError
            alertState = .init(.extraSetPinError(error.description))
        }
    }

    func setDecoyPin(_ pin: String) {
        sheetState = .none
        if auth.isInDecoyMode() { return isDecoyPinEnabled = true }

        do { try auth.rust.setDecoyPin(pin: pin) } catch {
            let error = error as! AuthManagerError
            alertState = .init(.extraSetPinError(error.description))
        }
    }
}

#Preview {
    SettingsContainer(route: .main)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
