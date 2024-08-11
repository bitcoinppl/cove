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
