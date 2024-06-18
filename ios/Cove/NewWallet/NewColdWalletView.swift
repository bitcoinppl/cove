//
//  NewColdWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewColdWalletView: View {
    var route: ColdWalletRoute

    var body: some View {
        switch route {
        case .create:
            Text("Create new cold wallet coming soon...")
        case .import:
            Text("Import new cold wallet coming soon...")
        }
    }
}

#Preview {
    NewColdWalletView(route: ColdWalletRoute.create)
}
