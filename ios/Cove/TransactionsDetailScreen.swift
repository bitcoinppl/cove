//
//  TransactionsDetailScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/27/24.
//

import SwiftUI

struct TransactionsDetailScreen: View {
    // public
    let transactionsDetails: TransactionDetails

    // private

    var body: some View {
        Text(/*@START_MENU_TOKEN@*/"Hello, World!"/*@END_MENU_TOKEN@*/)
    }
}

#Preview("confirmed") {
    TransactionsDetailScreen(transactionsDetails: TransactionDetails.previewNew())
}

#Preview("pending") {
    TransactionsDetailScreen(transactionsDetails: TransactionDetails.previewNewPending())
}
