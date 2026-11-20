enum MainSettingsSheetState: Equatable {
    case newPin
    case removePin
    case removeAllTrickPins
    indirect case removeWipeDataPin(TaggedItem<MainSettingsSheetState>? = .none)
    indirect case removeDecoyPin(TaggedItem<MainSettingsSheetState>? = .none)
    case changePin
    /// Decoy mode asks for the decoy PIN where main mode asks for its PIN, then applies the
    /// action to the toggles it shows
    case confirmDecoyPin(SecuritySettingsAction)
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
    case unverifiedDecoyWallets
    case unreadableWallets
    case cloudBackupKeyUnconfirmed
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

extension MainSettingsSheetState {
    /// The sheet for one Rust asks Settings to show, or nil when it asks for none
    init?(_ sheet: SecuritySheetState) {
        switch sheet {
        case .none: return nil
        case .newPin: self = .newPin
        case .removePin: self = .removePin
        case .changePin: self = .changePin
        case .enableBiometric: self = .enableBiometric
        case .disableBiometric: self = .disableBiometric
        case .enableWipeDataPin: self = .enableWipeDataPin
        case .removeWipeDataPin: self = .removeWipeDataPin()
        case .removeWipeDataPinThenEnableBiometric: self = .removeWipeDataPin(.init(.enableBiometric))
        case .enableDecoyPin: self = .enableDecoyPin
        case .removeDecoyPin: self = .removeDecoyPin()
        case .removeDecoyPinThenEnableBiometric: self = .removeDecoyPin(.init(.enableBiometric))
        case .removeAllTrickPins: self = .removeAllTrickPins
        }
    }
}

extension MainSettingsAlertState {
    /// The alert for one Rust asks Settings to show
    init(_ alert: SecurityAlertState) {
        switch alert {
        case let .unverifiedWallets(walletId): self = .unverifiedWallets(walletId)
        case .unverifiedDecoyWallets: self = .unverifiedDecoyWallets
        case .unreadableWallets: self = .unreadableWallets
        case .cloudBackupKeyUnconfirmed: self = .cloudBackupKeyUnconfirmed
        case .confirmEnableWipeMePin: self = .confirmEnableWipeMePin
        case .confirmDecoyPin: self = .confirmDecoyPin
        case .noteNoFaceIdWhenTrickPins: self = .noteNoFaceIdWhenTrickPins
        case .noteNoFaceIdWhenWipeMePin: self = .noteNoFaceIdWhenWipeMePin
        case .noteNoFaceIdWhenDecoyPin: self = .noteNoFaceIdWhenDecoyPin
        case .notePinRequired: self = .notePinRequired
        case .noteFaceIdDisablingForWipeMePin: self = .noteFaceIdDisabling(.confirmEnableWipeMePin)
        case .noteFaceIdDisablingForDecoyPin: self = .noteFaceIdDisabling(.confirmDecoyPin)
        case let .extraSetPinError(message): self = .extraSetPinError(message)
        }
    }
}
