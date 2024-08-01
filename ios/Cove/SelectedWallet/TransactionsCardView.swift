//
//  TransactionsList.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import SwiftUI

struct TransactionsCardView: View {
    let transactions: Transactions

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
            }
            .padding()
        }
        .background(.thickMaterial)
        .frame(maxHeight: .infinity)
    }
}

#Preview {
    TransactionsCardView(transactions: .empty())
}
