//
//  AppAlertState.swift
//  Cove
//
//  Created by Praveen Perera on 10/27/24.
//

public enum AppAlertState {
    case invalidWordGroup
    case duplicateWallet(WalletId)
    case errorImportingHotWallet(String)
    case importedSuccessfully
    case unableToSelectWallet
    case errorImportingHardwareWallet(String)
    case invalidFileFormat(String)
    case addressWrongNetwork(address: Address, network: Network, currentNetwork: Network)
    case noWalletSelected(Address)
    case foundAddress(Address)
    case noCameraPermission

    func title() -> String {
        switch self {
        case .invalidWordGroup:
            return "Words Not Valid"
        case .duplicateWallet:
            return "Duplicate Wallet"
        case .errorImportingHotWallet:
            return "Error"
        case .importedSuccessfully:
            return "Success"
        case .unableToSelectWallet:
            return "Error"
        case .errorImportingHardwareWallet:
            return "Error Importing Hardware Wallet"
        case .invalidFileFormat:
            return "Invalid File Format"
        case .addressWrongNetwork:
            return "Wrong Network"
        case .noWalletSelected:
            return "No Wallet Selected"
        case .foundAddress:
            return "Found Address"
        case .noCameraPermission:
            return "Camera Access is Required"
        }
    }
}
