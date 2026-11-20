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
            set: { enable in handleSecurityAction(.toggleBiometric(enable: enable)) }
        )
    }

    var togglePin: Binding<Bool> {
        Binding(
            get: {
                if auth.isInDecoyMode() { return isPinEnabled }
                return auth.type == AuthType.both || auth.type == AuthType.pin
            },
            set: { enable in handleSecurityAction(.togglePin(enable: enable)) }
        )
    }

    var toggleWipeMePin: Binding<Bool> {
        Binding(
            get: {
                if auth.isInDecoyMode() { return isWipeDataPinEnabled }
                return auth.isWipeDataPinEnabled
            },
            set: { enable in handleSecurityAction(.toggleWipeDataPin(enable: enable)) }
        )
    }

    var toggleDecoyPin: Binding<Bool> {
        Binding(
            get: {
                if auth.isInDecoyMode() { return isDecoyPinEnabled }
                return auth.isDecoyPinEnabled
            },
            set: { enable in handleSecurityAction(.toggleDecoyPin(enable: enable)) }
        )
    }

    /// Shows what Rust decides a security setting change needs, so iOS and Android follow the
    /// same rules, including what the decoy PIN's Settings do
    private func handleSecurityAction(_ action: SecuritySettingsAction) {
        switch auth.validateSecurityAction(action) {
        case let .proceedToSheet(sheet):
            sheetState = MainSettingsSheetState(sheet).map { .init($0) }
        case let .showAlert(alert):
            alertState = .init(MainSettingsAlertState(alert))
        case let .decoyPinThenLocalUpdate(action):
            sheetState = .init(.confirmDecoyPin(action))
        }
    }

    /// Decoy mode only pretends to change security settings, so it updates the toggles it shows
    private func applyDecoyModeLocalUpdate(_ action: SecuritySettingsAction) {
        switch action {
        case let .toggleBiometric(enable): isFaceIdEnabled = enable
        case let .togglePin(enable): isPinEnabled = enable
        case let .toggleWipeDataPin(enable): isWipeDataPinEnabled = enable
        case let .toggleDecoyPin(enable): isDecoyPinEnabled = enable
        case .changePin: break
        }
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
            applyDecoyModeLocalUpdate: applyDecoyModeLocalUpdate,
            isBetaImportExportEnabled: $isBetaImportExportEnabled,
            canUseBiometrics: canUseBiometrics,
            setPin: setPin,
            setWipeDataPin: setWipeDataPin,
            setDecoyPin: setDecoyPin
        )
    }

    var body: some View {
        MainSettingsForm(
            canUseBiometrics: canUseBiometrics(),
            toggleBiometric: toggleBiometric,
            togglePin: togglePin,
            toggleWipeMePin: toggleWipeMePin,
            toggleDecoyPin: toggleDecoyPin,
            trickPinNotice: auth.trickPinNotice,
            shouldShowBackupSection: shouldShowBackupSection,
            shouldShowCloudBackupSection: !auth.isInDecoyMode(),
            shouldShowBetaToggleSection: shouldShowBetaToggleSection,
            betaToggle: betaToggle,
            betaImportExportToggle: betaImportExportToggle,
            changePin: { handleSecurityAction(.changePin) },
            dismissTrickPinNotice: { auth.dispatch(action: .dismissTrickPinNotice) },
            exportAllBackups: exportAllBackups,
            importAllBackups: { sheetState = .init(.backupImport) },
            verifyBackup: { sheetState = .init(.backupVerify) },
            enableCloudBackup: { sheetState = .init(.cloudBackupOnboarding) },
            openCloudBackupDetail: { app.pushRoute(Route.settings(.cloudBackup)) }
        )
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
        .presentingSheetAndAlert(
            sheet: $sheetState,
            sheetContext: presentationContext,
            style: .fullScreenCover,
            alert: $alertState,
            alertContext: presentationContext,
            defaultTitle: "Error"
        )
    }

    // MARK: Setter functions

    func setPin(_ pin: String) {
        sheetState = .none
        if auth.isInDecoyMode() {
            isPinEnabled = true
            return
        }

        do { try auth.setPin(pin) } catch {
            alertState = .init(.extraSetPinError(error.localizedDescription))
        }
    }

    func setWipeDataPin(_ pin: String) {
        sheetState = .none
        if auth.isInDecoyMode() {
            isWipeDataPinEnabled = true
            return
        }

        do { try auth.rust.setWipeDataPin(pin: pin) } catch let AuthManagerError.WipeDataRefused(reason) {
            // a backup failed while the PIN sheet was open, so say why as the toggle does
            alertState = .init(MainSettingsAlertState(reason))
        } catch {
            alertState = .init(.extraSetPinError(error.localizedDescription))
        }
    }

    func setDecoyPin(_ pin: String) {
        sheetState = .none
        if auth.isInDecoyMode() {
            isDecoyPinEnabled = true
            return
        }

        do { try auth.rust.setDecoyPin(pin: pin) } catch {
            alertState = .init(.extraSetPinError(error.localizedDescription))
        }
    }
}

private struct MainSettingsForm: View {
    let canUseBiometrics: Bool
    @Binding var toggleBiometric: Bool
    @Binding var togglePin: Bool
    @Binding var toggleWipeMePin: Bool
    @Binding var toggleDecoyPin: Bool
    let trickPinNotice: DuplicateTrickPin?
    let shouldShowBackupSection: Bool
    let shouldShowCloudBackupSection: Bool
    let shouldShowBetaToggleSection: Bool
    @Binding var betaToggle: Bool
    @Binding var betaImportExportToggle: Bool
    let changePin: () -> Void
    let dismissTrickPinNotice: () -> Void
    let exportAllBackups: () -> Void
    let importAllBackups: () -> Void
    let verifyBackup: () -> Void
    let enableCloudBackup: () -> Void
    let openCloudBackupDetail: () -> Void

    var body: some View {
        Form {
            MainSettingsGeneralSection()
            WalletSettingsSection()
            MainSettingsSecuritySection(
                canUseBiometrics: canUseBiometrics,
                toggleBiometric: $toggleBiometric,
                togglePin: $togglePin,
                toggleWipeMePin: $toggleWipeMePin,
                toggleDecoyPin: $toggleDecoyPin,
                trickPinNotice: trickPinNotice,
                onChangePin: changePin,
                onDismissTrickPinNotice: dismissTrickPinNotice
            )
            MainSettingsBackupSection(
                isVisible: shouldShowBackupSection,
                exportAll: exportAllBackups,
                importAll: importAllBackups,
                verifyBackup: verifyBackup
            )
            MainSettingsCloudBackupSection(
                isVisible: shouldShowCloudBackupSection,
                onEnable: enableCloudBackup,
                onOpenDetail: openCloudBackupDetail
            )
            MainSettingsBetaToggleSection(
                isVisible: shouldShowBetaToggleSection,
                betaToggle: $betaToggle,
                betaImportExportToggle: $betaImportExportToggle
            )
            MainSettingsAboutSection()
        }
    }
}

private struct MainSettingsAboutSection: View {
    var body: some View {
        Section {
            SettingsRow(title: "About", route: .about, symbol: "info.circle")
        }
    }
}

#Preview {
    SettingsContainer(route: .main)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
        .environment(CloudBackupPresentationCoordinator())
}
