import Foundation

extension WalletAddressType: Comparable {
    public static func < (lhs: WalletAddressType, rhs: WalletAddressType) -> Bool {
        walletAddressTypeSortOrder(addressType: lhs) < walletAddressTypeSortOrder(addressType: rhs)
    }
}
