//
//  AppAlertState.swift
//  Cove
//
//  Created by Praveen Perera on 10/27/24.
//

public enum AppAlertState: Equatable {
    case invalidWordGroup
    case duplicateWallet(WalletId)
    case errorImportingHotWallet(String)
    case importedSuccessfully
    case unableToSelectWallet
    case errorImportingHardwareWallet(String)
    case invalidFileFormat(String)
    case addressWrongNetwork(address: Address, network: Network, currentNetwork: Network)
    case noWalletSelected(Address)
    case foundAddress(Address, Amount?)
    case noCameraPermission
    case failedToScanQr(error: String)
    case noUnsignedTransactionFound(TxId)
    case unableToGetAddress(error: String)

    func title() -> String {
        switch self {
        case .invalidWordGroup:
            "Words Not Valid"
        case .duplicateWallet:
            "Duplicate Wallet"
        case .errorImportingHotWallet:
            "Error"
        case .importedSuccessfully:
            "Success"
        case .unableToSelectWallet:
            "Error"
        case .errorImportingHardwareWallet:
            "Error Importing Hardware Wallet"
        case .invalidFileFormat:
            "Invalid File Format"
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
        }
    }
}
