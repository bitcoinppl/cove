//
//  ReceivedDetailsExpandedView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct ReceivedDetailsExpandedView: View {
    let model: WalletViewModel
    let transactionDetails: TransactionDetails
    let numberOfConfirmations: Int?

    // private
    @State private var isCopied = false

    @ViewBuilder
    func expandedDetailsRow(header: String, content: String) -> some View {
        Text(header)
            .font(.caption)
            .foregroundColor(.gray)
            .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)

        Text(content)
            .fontWeight(.semibold)
            .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)
            .padding(.bottom, 14)
    }

    var body: some View {
        VStack(alignment: .leading) {
            Divider().padding(.vertical, 18)

            if transactionDetails.isConfirmed() {
                Text("Confirmations")
                    .font(.caption)
                    .foregroundColor(.gray)
                    .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)

                if let numberOfConfirmations = self.numberOfConfirmations {
                    Text(ThousandsFormatter(numberOfConfirmations).fmt())
                        .fontWeight(.semibold)
                        .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)
                        .padding(.bottom, 14)
                } else {
                    ProgressView()
                }

                expandedDetailsRow(header: "Block Number", content: String(transactionDetails.blockNumberFmt() ?? ""))
            }

            Text("Received At")
                .font(.caption)
                .foregroundColor(.gray)
                .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)

            HStack {
                Text(transactionDetails.addressSpacedOut())
                    .fontWeight(.semibold)
                    .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)
                    .padding(.bottom, 14)

                Spacer()
                Spacer()

                Button(action: {
                    UIPasteboard.general.string = transactionDetails.address().string()
                    withAnimation {
                        isCopied = true
                    }

                    // Reset the button text after a delay
                    DispatchQueue.main.asyncAfter(deadline: .now() + 5) {
                        withAnimation {
                            isCopied = false
                        }
                    }
                }) {
                    HStack(spacing: 8) {
                        Image(systemName: "doc.on.doc")
                            .font(.caption)

                        Text(isCopied ? "Copied" : "Copy")
                            .font(.caption)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(Color.white)
                    .foregroundColor(.black)
                    .overlay(
                        RoundedRectangle(cornerRadius: 20)
                            .stroke(Color.gray.opacity(0.3), lineWidth: 1)
                    )
                }
                .buttonStyle(PlainButtonStyle())
            }
        }
        .padding(.horizontal, detailsExpandedPadding)
    }
}
