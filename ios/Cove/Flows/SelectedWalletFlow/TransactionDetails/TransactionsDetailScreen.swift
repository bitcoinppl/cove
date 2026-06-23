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
    let txId: TxId

    var body: some View {
        WalletManagerHost(walletId: id, loading: {
            FullPageLoadingView(backgroundColor: .background)
        }, onError: { error in
            Log.error("Something went very wrong: \(error)")
            app.trySelectLatestOrNewWallet()
        }) { manager in
            TransactionDetailsLoader(id: id, txId: txId, manager: manager)
        }
    }
}

private struct TransactionDetailsLoader: View {
    let id: WalletId
    let txId: TxId
    var manager: WalletManager

    @State private var error: Error?
    @State private var didLoadInitialDetails = false

    private var transactionDetails: TransactionDetails? {
        manager.transactionDetails[txId]
    }

    var body: some View {
        Group {
            if let transactionDetails {
                TransactionDetailsView(
                    id: id,
                    txId: txId,
                    transactionDetails: transactionDetails,
                    refreshOnAppear: !didLoadInitialDetails,
                    manager: manager
                )
            } else if let error {
                TransactionDetailsLoadErrorView(error: error) {
                    Task { await loadTransactionDetails() }
                }
            } else {
                FullPageLoadingView(backgroundColor: .background)
            }
        }
        .task(id: txId) {
            await loadTransactionDetails()
        }
    }

    private func loadTransactionDetails() async {
        if transactionDetails != nil { return }

        await MainActor.run {
            error = nil
        }

        do {
            _ = try await manager.transactionDetails(for: txId)
            await MainActor.run {
                didLoadInitialDetails = true
            }
        } catch {
            await MainActor.run {
                self.error = error
            }

            Log.error("Unable to get transaction details: \(error.localizedDescription), for txn: \(txId)")
        }
    }
}

private struct TransactionDetailsLoadErrorView: View {
    let error: Error
    let retry: () -> Void

    var body: some View {
        ContentUnavailableView {
            Label("Unable to Load Transaction", systemImage: "exclamationmark.triangle")
        } description: {
            Text(error.localizedDescription)
        } actions: {
            Button("Try Again", action: retry)
                .buttonStyle(.borderedProminent)
        }
    }
}
