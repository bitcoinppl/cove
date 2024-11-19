//
//  TransactionsDetailScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/27/24.
//

import SwiftUI

let detailsExpandedPadding: CGFloat = 28

struct TransactionsDetailScreen: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    // public
    let id: WalletId
    let transactionDetails: TransactionDetails

    // private
    @State var model: WalletViewModel? = nil

    func loadModel() {
        if model != nil { return }
        if model != nil { return }

        do {
            Log.debug("Getting wallet model for \(id)")
            model = try app.getWalletViewModel(id: id)
        } catch {
            Log.error("Something went very wrong: \(error)")
            navigate(Route.listWallets)
        }
    }

    var body: some View {
        Group {
            if let model {
                TransactionDetailsView(id: id, transactionDetails: transactionDetails, model: model)
            } else {
                Text("Loading...")
            }
        }
        .task {
            loadModel()
        }
    }
}
