//
//  SentDetailsExpandedView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct SentDetailsExpandedView: View {
    let model: WalletViewModel
    let transactionDetails: TransactionDetails

    var metadata: WalletMetadata {
        model.walletMetadata
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Divider().padding(.vertical, 18)

            VStack(alignment: .leading, spacing: 8) {
                Text("Sent to")
                    .font(.footnote)
                    .foregroundColor(.secondary)
                    .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)

                Text(transactionDetails.addressSpacedOut())
                    .fontWeight(.semibold)
                    .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)
                    .textSelection(.enabled)

                if transactionDetails.isConfirmed() {
                    HStack(spacing: 0) {
                        Group {
                            Text(transactionDetails.blockNumberFmt() ?? "")
                            Text("|")

                            AsyncView(operation: {
                                let blockNumber = transactionDetails.blockNumber() ?? 0
                                return try await model.rust.numberOfConfirmationsFmt(blockHeight: blockNumber)
                            }) { (confirmations: String) in
                                Group {
                                    Text(confirmations)

                                    Image(systemName: "checkmark.circle.fill")
                                        .font(.system(size: 10))
                                        .fontWeight(.bold)
                                        .foregroundStyle(.green)
                                        .padding(.leading, 3)
                                }
                            }
                        }

                        .padding(.horizontal, 2)
                    }
                    .font(.caption).foregroundStyle(.tertiary)
                }
            }

            Divider().padding(.vertical, 18)

            HStack(alignment: .top) {
                Text("Network Fee")
                Image(systemName: "info.circle")
                    .font(.footnote)
                    .fontWeight(/*@START_MENU_TOKEN@*/ .bold/*@END_MENU_TOKEN@*/)
                    .foregroundStyle(.tertiary.opacity(0.8))
                Spacer()

                VStack(alignment: .trailing) {
                    Text(transactionDetails.feeFmt(unit: metadata.selectedUnit) ?? "")
                    AsyncView(operation: transactionDetails.feeFiatFmt) { amount in
                        Text("≈ $\(amount) USD").foregroundStyle(.secondary)
                            .font(.caption)
                            .padding(.top, 2)
                    }
                }
            }
            .font(.subheadline)
            .foregroundStyle(.secondary)

            HStack(alignment: .top) {
                Text("Receipient Receives")
                Spacer()

                VStack(alignment: .trailing) {
                    Text(transactionDetails.sentSansFeeFmt(unit: metadata.selectedUnit) ?? "")
                    AsyncView(operation: transactionDetails.sentSansFeeFiatFmt) { amount in
                        Text("≈ $\(amount) USD").foregroundStyle(.secondary)
                            .font(.caption)
                            .padding(.top, 2)
                    }
                }
            }
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .padding(.top, 12)

            Divider().padding(.vertical, 18)

            HStack(alignment: .top) {
                Text("Total Spent")

                Spacer()
                VStack(alignment: .trailing) {
                    Text(transactionDetails.amountFmt(unit: metadata.selectedUnit))
                    AsyncView(operation: transactionDetails.amountFiatFmt) { amount in
                        Text("≈ $\(amount) USD").foregroundStyle(.secondary)
                            .font(.caption)
                            .padding(.top, 2)
                    }
                }
            }
            .font(.subheadline)
        }
        .padding(.horizontal, detailsExpandedPadding)
    }
}
