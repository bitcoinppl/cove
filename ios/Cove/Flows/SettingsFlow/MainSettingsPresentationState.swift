enum MainSettingsSheetState: Equatable {
    case newPin
    case removePin
    case removeAllTrickPins
    indirect case removeWipeDataPin(TaggedItem<MainSettingsSheetState>? = .none)
    indirect case removeDecoyPin(TaggedItem<MainSettingsSheetState>? = .none)
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

enum MainSettingsAlertState: Equatable {
    case unverifiedWallets(WalletId)
    case confirmEnableWipeMePin
    case confirmDecoyPin
    case noteNoFaceIdWhenTrickPins
    case noteNoFaceIdWhenWipeMePin
    case noteNoFaceIdWhenDecoyPin
    case notePinRequired
    indirect case noteFaceIdDisabling(MainSettingsAlertState)
    case confirmBetaImportExport
    case extraSetPinError(String)
}
