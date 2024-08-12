//
//  TransactionsCardView.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import SwiftUI

struct TransactionsCardView: View {
    let transactions: [Transaction]
    let scanComplete: Bool

    @ViewBuilder
    func TransactionRow(_ txn: Transaction) -> some View {
        switch txn {
        case let .confirmed(txn):
            ConfirmedTransactionView(transaction: txn)
        case let .unconfirmed(txn):
            UnconfirmedTransactionView(transaction: txn)
        }
    }

    var body: some View {
        VStack {
            VStack {
                HStack {
                    Text("Transactions")
                        .foregroundStyle(.secondary)
                        .font(.subheadline)
                        .fontWeight(.bold)
                    Spacer()
                }

                ForEach(transactions, content: TransactionRow)

                if transactions.isEmpty {
                    ContentUnavailableView {
                        Label("No transactions", systemImage: "bitcoinsign.square.fill")
                    } description: {
                        Text("Send some bitcoin to yourself")
                    }
                }
            }
            .padding()
            .padding(.top, 5)
        }
    }
}

struct ConfirmedTransactionView: View {
    let transaction: ConfirmedTransaction

    var body: some View {
        HStack {
            Text("Confirmed")
            Text(String(transaction.blockHeight()))
            Text(String(transaction.confirmedAt()))
        }
    }
}

struct UnconfirmedTransactionView: View {
    let transaction: UnconfirmedTransaction

    var body: some View {
        HStack {
            Text("Unconfirmed")
        }
    }
}

#Preview("Full of Txns - Complete") {
    TransactionsCardView(
        transactions: transactionsPreviewNew(confirmed: UInt8(25), unconfirmed: UInt8(1)),
        scanComplete: true
    )
}

#Preview("Full of Txns - Scanning") {
    TransactionsCardView(
        transactions: transactionsPreviewNew(confirmed: UInt8(25), unconfirmed: UInt8(1)),
        scanComplete: false
    )
}

#Preview("Empty - Scanning") {
    TransactionsCardView(transactions: [], scanComplete: false)
}

#Preview("Empty") {
    TransactionsCardView(transactions: [], scanComplete: true)
}
