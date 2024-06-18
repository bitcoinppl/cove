//
//  NewHotWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewHotWalletView: View {
    var route: HotWalletRoute

    var body: some View {
        switch route {
        case .select:
            HotWalletSelectView()
        case .create:
            Text("Create a new hot wallet...")
        case .import:
            Text("Import a new hot wallet...")
        }
    }
}

#Preview("Select") {
    NewHotWalletView(route: HotWalletRoute.select)
}

#Preview("Create") {
    NewHotWalletView(route: HotWalletRoute.create)
}

#Preview("Import") {
    NewHotWalletView(route: HotWalletRoute.import)
}
