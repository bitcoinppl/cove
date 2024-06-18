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
        case .create:
            Text("Create a new hot wallet...")
        case .import:
            Text("Import a new hot wallet...")
        }
    }
}

#Preview {
    NewHotWalletView(route: HotWalletRoute.create)
}
