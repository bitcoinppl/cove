import SwiftUI

struct MainSettingsPresentationContext {
    let app: AppManager
    let auth: AuthManager
    let sheetState: Binding<TaggedItem<MainSettingsSheetState>?>
    let alertState: Binding<TaggedItem<MainSettingsAlertState>?>
    let isPinEnabled: Binding<Bool>
    let isBetaImportExportEnabled: Binding<Bool>
    let canUseBiometrics: () -> Bool
    let setPin: (String) -> Void
    let setWipeDataPin: (String) -> Void
    let setDecoyPin: (String) -> Void

    func dismissAlert() {
        alertState.wrappedValue = .none
    }

    func dismissSheet() {
        sheetState.wrappedValue = .none
    }

    func presentAlert(_ alert: MainSettingsAlertState) {
        alertState.wrappedValue = .init(alert)
    }

    func presentSheet(_ sheet: MainSettingsSheetState) {
        sheetState.wrappedValue = .init(sheet)
    }

    func setSheet(_ sheet: TaggedItem<MainSettingsSheetState>?) {
        sheetState.wrappedValue = sheet
    }
}
