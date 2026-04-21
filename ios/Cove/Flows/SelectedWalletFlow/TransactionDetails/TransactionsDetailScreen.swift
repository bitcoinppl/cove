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

    // public
    let id: WalletId
    let transactionDetails: TransactionDetails

    var body: some View {
        WalletManagerHost(walletId: id, loading: {
            Text("Loading...")
        }, onError: { error in
            Log.error("Something went very wrong: \(error)")
            app.trySelectLatestOrNewWallet()
        }) { manager in
            TransactionDetailsView(
                id: id, transactionDetails: transactionDetails, manager: manager
            )
        }
    }
}
