//
//  AppAlertState.swift
//  Cove
//
//  Created by Praveen Perera on 10/27/24.
//

public enum AppAlertState: Equatable {
    // success
    case importedSuccessfully
    case importedLabelsSuccessfully

    /// warn
    case duplicateWallet(WalletId)

    // errors
    case invalidWordGroup
    case errorImportingHotWallet(String)
    case addressWrongNetwork(address: Address, network: Network, currentNetwork: Network)
    case foundAddress(Address, Amount?)
    case unableToSelectWallet
    case errorImportingHardwareWallet(String)
    case invalidFileFormat(String)
    case noWalletSelected(Address)
    case invalidFormat(String)
    case noUnsignedTransactionFound(TxId)
    case unableToGetAddress(error: String)
    case noCameraPermission
    case failedToScanQr(error: String)
    case cantSendOnWatchOnlyWallet
    case tapSignerSetupFailed(String)
    case tapSignerDeriveFailed(String)
    case tapSignerInvalidAuth
    case tapSignerNoBackup(TapSigner)
    case tapSignerWrongPin(TapSigner, AfterPinAction)

    /// genericMessage or error
    case general(title: String, message: String)

    // action
    case uninitializedTapSigner(TapSigner)
    case tapSignerWalletFound(WalletId)
    case intializedTapSigner(TapSigner)

    func title() -> String {
        switch self {
        case .invalidWordGroup:
            "Words Not Valid"
        case .duplicateWallet:
            "Duplicate Wallet"
        case .errorImportingHotWallet:
            "Error"
        case .importedSuccessfully, .importedLabelsSuccessfully:
            "Success"
        case .unableToSelectWallet:
            "Error"
        case .errorImportingHardwareWallet:
            "Error Importing Hardware Wallet"
        case .invalidFileFormat:
            "Invalid File Format"
        case .invalidFormat:
            "Invalid Format"
        case .addressWrongNetwork:
            "Wrong Network"
        case .noWalletSelected,
             .foundAddress:
            "Found Address"
        case .noCameraPermission:
            "Camera Access is Required"
        case .failedToScanQr:
            "Failed to Scan QR"
        case .noUnsignedTransactionFound:
            "No Unsigned Transaction Found"
        case .unableToGetAddress:
            "Unable to Get Address"
        case .cantSendOnWatchOnlyWallet:
            "Watch Only Wallet"
        case .uninitializedTapSigner:
            "Setup TAPSIGNER?"
        case .tapSignerSetupFailed:
            "Setup Failed"
        case .tapSignerDeriveFailed:
            "TAPSIGNER Import Failed"
        case .tapSignerInvalidAuth:
            "Wrong PIN"
        case .tapSignerWalletFound:
            "Wallet Found"
        case .intializedTapSigner:
            "Import TAPSIGNER?"
        case .tapSignerNoBackup:
            "No Backup Found"
        case .tapSignerWrongPin:
            "Wrong PIN"
        case let .general(title: title, message: _):
            title
        }
    }
}
