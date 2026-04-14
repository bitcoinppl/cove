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
    case backupExport
    case backupImport
    case backupVerify
    case backupExportAuth
    case cloudBackupOnboarding
}

private enum AlertState: Equatable {
    case unverifiedWallets(WalletId)
    case confirmEnableWipeMePin
    case confirmDecoyPin
    case noteNoFaceIdWhenTrickPins
    case noteNoFaceIdWhenWipeMePin
    case noteNoFaceIdWhenDecoyPin
    case notePinRequired
    indirect case noteFaceIdDisabling(AlertState)
    case confirmBetaImportExport
    case extraSetPinError(String)
}

struct MainSettingsScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
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

    /// beta features
    @State private var isBetaEnabled = Database().globalFlag().getBoolConfig(key: .betaFeaturesEnabled)
    @State private var isBetaImportExportEnabled = Database().globalFlag().getBoolConfig(key: .betaImportExportEnabled)

    let themes = allColorSchemes()

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
                if auth.isInDecoyMode() {
                    isFaceIdEnabled = enable
                    return
                }

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
                            .unverifiedWallets(app.rust.unverifiedWalletIds().first!)
                        )

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
                if !enable, auth.isInDecoyMode() {
                    isDecoyPinEnabled = false
                    return
                }

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

    @ViewBuilder
    var BackupSection: some View {
        if isBetaEnabled, isBetaImportExportEnabled, !auth.isInDecoyMode() {
            Section(header: HStack(spacing: 6) {
                Text("Backup")
                Text("BETA")
                    .font(.caption2)
                    .fontWeight(.semibold)
                    .foregroundStyle(.white)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(.orange, in: Capsule())
            }) {
                SettingsRow(title: "Export All", symbol: "square.and.arrow.up") {
                    if auth.type != .none {
                        sheetState = .init(.backupExportAuth)
                    } else {
                        sheetState = .init(.backupExport)
                    }
                }

                SettingsRow(title: "Import All", symbol: "square.and.arrow.down") {
                    sheetState = .init(.backupImport)
                }

                SettingsRow(title: "Verify Backup", symbol: "checkmark.shield") {
                    sheetState = .init(.backupVerify)
                }
            }
        }
    }

    @ViewBuilder
    var CloudBackupSection: some View {
        if !auth.isInDecoyMode() {
            let manager = CloudBackupManager.shared

            Section(header: Text("Cloud Backup")) {
                switch manager.status {
                case .disabled:
                    SettingsRow(title: "Enable Cloud Backup", symbol: "icloud.and.arrow.up") {
                        sheetState = .init(.cloudBackupOnboarding)
                    }
                case .enabling:
                    cloudBackupEnablingRow
                case .enabled:
                    cloudBackupEnabledRow(manager: manager)
                case .passkeyMissing:
                    cloudBackupPasskeyMissingRow
                case .unsupportedPasskeyProvider:
                    cloudBackupUnsupportedProviderRow
                case .restoring:
                    cloudBackupRestoringRow
                case let .error(message):
                    cloudBackupErrorContent(message: message, manager: manager)
                }
            }
        }
    }

    private var cloudBackupEnablingRow: some View {
        HStack {
            SettingsIcon(symbol: "icloud.and.arrow.up")
            Text("Setting up cloud backup...")
                .font(.subheadline)
                .padding(8)
            Spacer()
            ProgressView()
        }
    }

    private func cloudBackupEnabledRow(manager: CloudBackupManager) -> some View {
        HStack {
            cloudBackupEnabledStatus(manager: manager)
            Spacer()
            settingsChevron
        }
        .contentShape(Rectangle())
        .onTapGesture {
            app.pushRoute(Route.settings(.cloudBackup))
        }
    }

    @ViewBuilder
    private func cloudBackupEnabledStatus(manager: CloudBackupManager) -> some View {
        if manager.isUnverified {
            Image(systemName: "exclamationmark.icloud")
                .foregroundStyle(.orange)
            Text("Cloud Backup Unverified")
        } else if manager.hasPendingUploadVerification {
            Image(systemName: "arrow.clockwise.icloud")
                .foregroundStyle(.blue)
            Text("Cloud Backup Verifying")
        } else {
            Image(
                systemName: manager.isVerificationStale
                    ? "exclamationmark.icloud" : "checkmark.icloud"
            )
            .foregroundStyle(manager.isVerificationStale ? .orange : .green)

            VStack(alignment: .leading, spacing: 2) {
                Text("Cloud Backup Enabled")

                if manager.isVerificationStale {
                    Text("Verification recommended")
                        .font(.caption2)
                        .foregroundStyle(.orange)
                }
            }
        }
    }

    private var cloudBackupPasskeyMissingRow: some View {
        cloudBackupActionRow(
            icon: "exclamationmark.icloud.fill",
            title: "Cloud Backup Passkey Missing",
            message: "Backups can't be restored until you add a new passkey"
        )
    }

    private var cloudBackupUnsupportedProviderRow: some View {
        cloudBackupActionRow(
            icon: "exclamationmark.shield.fill",
            title: "Supported Password Manager Required",
            message: "Use Apple Passwords, 1Password, or Bitwarden"
        )
    }

    private func cloudBackupActionRow(icon: String, title: String, message: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: icon)
                .foregroundStyle(.red)

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .foregroundStyle(.red)
                    .fontWeight(.semibold)
                    .lineLimit(1)

                Text(message)
                    .font(.caption2)
                    .foregroundStyle(.red.opacity(0.5))
                    .lineLimit(1)
            }

            Spacer()
            settingsChevron
        }
        .contentShape(Rectangle())
        .onTapGesture {
            app.pushRoute(Route.settings(.cloudBackup))
        }
    }

    private var cloudBackupRestoringRow: some View {
        HStack {
            ProgressView()
                .padding(.trailing, 8)
            Text("Restoring from cloud backup...")
        }
    }

    private func cloudBackupErrorContent(message: String, manager: CloudBackupManager) -> some View {
        Group {
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Image(systemName: "exclamationmark.icloud")
                        .foregroundStyle(.red)
                    Text("Cloud Backup Error")
                }
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            SettingsRow(title: "Retry", symbol: "arrow.clockwise") {
                manager.dispatch(action: .enableCloudBackup)
            }
        }
    }

    private var settingsChevron: some View {
        Image(systemName: "chevron.right")
            .foregroundColor(Color(UIColor.tertiaryLabel))
            .font(.footnote)
            .fontWeight(.semibold)
    }

    var betaToggle: Binding<Bool> {
        Binding(
            get: { isBetaEnabled },
            set: { newValue in
                try? Database().globalFlag().set(key: .betaFeaturesEnabled, value: newValue)
                isBetaEnabled = newValue

                if !newValue {
                    try? Database().globalFlag().set(key: .betaImportExportEnabled, value: false)
                    isBetaImportExportEnabled = false
                }
            }
        )
    }

    var betaImportExportToggle: Binding<Bool> {
        Binding(
            get: { isBetaImportExportEnabled },
            set: { newValue in
                if newValue {
                    alertState = .init(.confirmBetaImportExport)
                } else {
                    try? Database().globalFlag().set(key: .betaImportExportEnabled, value: false)
                    isBetaImportExportEnabled = false
                }
            }
        )
    }

    private var hasCloudBackupPresentationBlocker: Bool {
        sheetState != nil || alertState != nil
    }

    @ViewBuilder
    var BetaToggleSection: some View {
        if isBetaEnabled, !auth.isInDecoyMode() {
            Section {
                Toggle("Beta Features", isOn: betaToggle)
                Toggle("Enable Beta Import Export", isOn: betaImportExportToggle)
            } footer: {
                Text("Disable to hide experimental features")
            }
        }
    }

    var body: some View {
        Form {
            GeneralSection
            WalletSettingsSection()
            SecuritySection
            BackupSection
            CloudBackupSection
            BetaToggleSection

            Section {
                SettingsRow(title: "About", route: .about, symbol: "info.circle")
            }
        }
        .scrollContentBackground(.hidden)
        .onAppear {
            isBetaEnabled = Database().globalFlag().getBoolConfig(key: .betaFeaturesEnabled)
            isBetaImportExportEnabled = Database().globalFlag().getBoolConfig(key: .betaImportExportEnabled)
        }
        .onDisappear {
            cloudBackupPresentationCoordinator.setBlocker(.settingsLocalModal, active: false)
        }
        .onChange(of: hasCloudBackupPresentationBlocker, initial: true) { _, active in
            cloudBackupPresentationCoordinator.setBlocker(.settingsLocalModal, active: active)
        }
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
        .fullScreenCover(item: $sheetState, content: SheetContent)
        .alert(
            alertTitle,
            isPresented: showingAlert,
            presenting: alertState,
            actions: { MyAlert($0).actions },
            message: { MyAlert($0).message }
        )
    }

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

        case .confirmBetaImportExport:
            AlertBuilder(
                title: "Experimental Feature",
                message: "This is a very experimental feature. Use with caution. This is mostly used by developers for testing purposes.",
                actions: {
                    Button("Accept") {
                        try? Database().globalFlag().set(key: .betaImportExportEnabled, value: true)
                        isBetaImportExportEnabled = true
                        alertState = .none
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
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
                        isPinEnabled = false
                        return
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
                    if auth.isInDecoyMode() {
                        sheetState = .none
                        return
                    }

                    sheetState = .none
                    if auth.checkWipeDataPin(pin) {
                        alertState = .init(
                            .extraSetPinError(
                                "Can't update PIN because its the same as your wipe data PIN"
                            )
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

        case .backupExportAuth:
            LockView(
                lockType: auth.type,
                isPinCorrect: { pin in
                    if auth.isInDecoyMode() { return auth.checkDecoyPin(pin) }
                    return auth.checkPin(pin)
                },
                onUnlock: { _ in
                    if auth.isInDecoyMode() { sheetState = .none; return }
                    sheetState = .init(.backupExport)
                },
                backAction: { sheetState = .none },
                content: { EmptyView() }
            )

        case .backupExport:
            NavigationStack {
                BackupExportView()
                    .navigationTitle("Export Backup")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { sheetState = .none }
                        }
                    }
            }

        case .backupImport:
            NavigationStack {
                BackupImportView()
                    .navigationTitle("Import Backup")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { sheetState = .none }
                        }
                    }
            }

        case .backupVerify:
            NavigationStack {
                BackupVerifyView()
                    .navigationTitle("Verify Backup")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { sheetState = .none }
                        }
                    }
            }

        case .cloudBackupOnboarding:
            CloudBackupEnableOnboardingView(
                onEnable: {
                    sheetState = .none
                    CloudBackupManager.shared.dispatch(action: .enableCloudBackup)
                },
                onCancel: { sheetState = .none }
            )
        }
    }

    // MARK: Setter functions

    func setPin(_ pin: String) {
        if auth.isInDecoyMode() {
            isPinEnabled = true
            return
        }
        auth.dispatch(action: .setPin(pin))
        sheetState = .none
    }

    func setWipeDataPin(_ pin: String) {
        sheetState = .none
        if auth.isInDecoyMode() {
            isWipeDataPinEnabled = true
            return
        }

        do { try auth.rust.setWipeDataPin(pin: pin) } catch {
            let error = error as! AuthManagerError
            alertState = .init(.extraSetPinError(error.description))
        }
    }

    func setDecoyPin(_ pin: String) {
        sheetState = .none
        if auth.isInDecoyMode() {
            isDecoyPinEnabled = true
            return
        }

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
        .environment(CloudBackupPresentationCoordinator())
}
