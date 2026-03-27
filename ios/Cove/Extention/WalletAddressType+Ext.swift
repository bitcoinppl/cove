import Foundation

extension WalletAddressType: Comparable {
    public static func < (lhs: WalletAddressType, rhs: WalletAddressType) -> Bool {
        lhs.sortOrder() < rhs.sortOrder()
    }
}
