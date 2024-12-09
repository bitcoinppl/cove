//
//  WalletSettingsContainer.swift
//  Cove
//
//  Created by Praveen Perera on 12/5/24.
//

import Foundation
import SwiftUI

struct WalletSettingsContainer: View {
    @Environment(AppManager.self) var app

    // args
    let id: WalletId

    // private
    @State private var manager: WalletManager? = nil
    @State private var error: String? = nil

    func initOnAppear() {
        do {
            let manager = try app.getWalletManager(id: id)
            self.manager = manager
        } catch {
            self.error = "Failed to get wallet \(error.localizedDescription)"
            Log.error(self.error!)
        }
    }

    var body: some View {
        if let manager {
            WalletSettingsSheet(manager: manager, isSheet: false)
        } else {
            Text(error ?? "Loading...")
                .task {
                    guard let error else { return }
                    Log.error(error)
                    try? await Task.sleep(for: .seconds(5))
                    app.resetRoute(to: .listWallets)
                }
                .onAppear(perform: initOnAppear)
        }
    }
}
