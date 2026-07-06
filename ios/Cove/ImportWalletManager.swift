import SwiftUI

@Observable class ImportWalletManager {
    var rust: RustImportWalletManager

    public init() {
        rust = RustImportWalletManager()
    }
}
