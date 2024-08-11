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
        }
        .background(.thickMaterial)
        .frame(maxHeight: .infinity)
    }
}

#Preview("Empty") {
    TransactionsCardView(transactions: [], scanComplete: false)
}
