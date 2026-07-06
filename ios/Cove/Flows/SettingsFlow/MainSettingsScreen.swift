import LocalAuthentication
import SwiftUI

struct MainSettingsScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(CloudBackupPresentationCoordinator.self)
    private var cloudBackupPresentationCoordinator
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme

    // private
    @State private var sheetState: TaggedItem<MainSettingsSheetState>? = nil
    @State private var alertState: TaggedItem<MainSettingsAlertState>? = nil

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

    private var shouldShowBackupSection: Bool {
        isBetaEnabled && isBetaImportExportEnabled && !auth.isInDecoyMode()
    }

    private var shouldShowBetaToggleSection: Bool {
        isBetaEnabled && !auth.isInDecoyMode()
    }

    private func exportAllBackups() {
        if auth.type != .none {
            sheetState = .init(.backupExportAuth)
        } else {
            sheetState = .init(.backupExport)
        }
    }

    private var presentationContext: MainSettingsPresentationContext {
        MainSettingsPresentationContext(
            app: app,
            auth: auth,
            sheetState: $sheetState,
            alertState: $alertState,
            isPinEnabled: $isPinEnabled,
            isBetaImportExportEnabled: $isBetaImportExportEnabled,
            canUseBiometrics: canUseBiometrics,
            setPin: setPin,
            setWipeDataPin: setWipeDataPin,
            setDecoyPin: setDecoyPin
        )
    }

    var body: some View {
        Form {
            MainSettingsGeneralSection()
            WalletSettingsSection()
            MainSettingsSecuritySection(
                canUseBiometrics: canUseBiometrics(),
                toggleBiometric: toggleBiometric,
                togglePin: togglePin,
                toggleWipeMePin: toggleWipeMePin,
                toggleDecoyPin: toggleDecoyPin
            ) {
                sheetState = .init(.changePin)
            }
            MainSettingsBackupSection(
                isVisible: shouldShowBackupSection,
                exportAll: exportAllBackups,
                importAll: { sheetState = .init(.backupImport) },
                verifyBackup: { sheetState = .init(.backupVerify) }
            )
            MainSettingsCloudBackupSection(
                isVisible: !auth.isInDecoyMode(),
                onEnable: {
                    sheetState = .init(.cloudBackupOnboarding)
                },
                onOpenDetail: {
                    app.pushRoute(Route.settings(.cloudBackup))
                }
            )
            MainSettingsBetaToggleSection(
                isVisible: shouldShowBetaToggleSection,
                betaToggle: betaToggle,
                betaImportExportToggle: betaImportExportToggle
            )

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
        .presentingFullScreenCover($sheetState, context: presentationContext)
        .presentingAlert($alertState, context: presentationContext, defaultTitle: "Error")
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
