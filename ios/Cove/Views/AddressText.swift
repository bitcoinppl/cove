//
//  AddressText.swift
//  Cove
//

import SwiftUI

extension Text {
    /// Displays a bitcoin address in the shared format, 4 character groups in a monospaced font
    init(address: Address) {
        self.init(addressString: address.unformatted())
    }

    /// Displays an address string in the shared format, already grouped input is regrouped identically
    init(addressString: String) {
        self = Text(verbatim: addressString.addressSpacedOut()).monospaced()
    }
}
