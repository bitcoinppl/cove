//
//  CoinControlContainer.swift
//  Cove
//
//  Created by Praveen Perera on 5/19/25.
//

import Foundation
import SwiftUI

struct CoinControlContainer: View {
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate

    let route: CoinControlRoute

    // private
    @State var walletManager: WalletManager? = nil
    @State var manager: CoinControlManager? = nil

    func initOnAppear() {
        let id = route.id()
        if walletManager != nil, manager != nil { return }

        Task {
            do {
                Log.debug("Getting wallet for CoinControlRoute \(id)")
                let walletManager = try app.getWalletManager(id: id)
                let rustManager = await walletManager.rust.newCoinControlManager()
                let manager = CoinControlManager(rustManager)

                self.walletManager = walletManager
                self.manager = manager
            }
        }
    }

    var body: some View {
        if let walletManager, let manager {
            switch route {
            case .list:
                UtxoListScreen(manager: manager)
                    .environment(walletManager)
            }
        } else {
            ProgressView()
                .onAppear(perform: initOnAppear)
        }
    }
}
