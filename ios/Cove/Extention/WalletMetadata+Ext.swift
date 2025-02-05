import Foundation
import SwiftUI

extension WalletMetadata: Identifiable & Hashable & Equatable {
    var swiftColor: Color {
        Color(color)
    }

    public static func == (lhs: WalletMetadata, rhs: WalletMetadata) -> Bool {
        lhs.id == rhs.id
    }

    public func hash(into hasher: inout Hasher) {
        hasher.combine(id)
    }

    #if DEBUG
        init(_ name: String = "Test Wallet", preview: Bool) {
            assert(preview)
            self = walletMetadataPreview()
            self.name = name
        }
    #endif
}
