import Foundation
import SwiftUI

extension WalletMetadata {
    var swiftColor: Color {
        Color(color)
    }

    #if DEBUG
        init(_ name: String = "Test Wallet", preview: Bool) {
            assert(preview)
            self = walletMetadataPreview()
            self.name = name
        }
    #endif
}
