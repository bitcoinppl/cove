//
//  NewWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewWalletView: View {
    var route: NewWalletRoute
    @Environment(MainViewModel.self) private var mainViewModel

    var body: some View {
        switch route {
        case .select:
            NewWalletSelect()
        case let .hotWallet(route):
            NewHotWalletView(route: route)
        case let .coldWallet(route):
            NewColdWalletView(route: route)
        }
    }
}

#Preview {
    NewWalletView(route: NewWalletRoute.select).environment(MainViewModel())
}
