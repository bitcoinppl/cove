//
//  TransactionsDetailScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/27/24.
//

import SwiftUI

let detailsExpandedPadding: CGFloat = 28

struct TransactionsDetailScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate

    // public
    let id: WalletId
    let transactionDetails: TransactionDetails

    // private
    @State var manager: WalletManager? = nil

    func loadManager() {
        if manager != nil { return }
        if manager != nil { return }

        do {
            Log.debug("Getting wallet manager for \(id)")
            manager = try app.getWalletManager(id: id)
        } catch {
            Log.error("Something went very wrong: \(error)")
            app.rust.selectLatestOrNewWallet()
        }
    }

    var body: some View {
        Group {
            if let manager {
                TransactionDetailsView(
                    id: id, transactionDetails: transactionDetails, manager: manager
                )
            } else {
                Text("Loading...")
            }
        }
        .task { loadManager() }
    }
}
