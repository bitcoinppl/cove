//
//  SentDetailsExpandedView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct SentDetailsExpandedView: View {
    let manager: WalletManager
    let transactionDetails: TransactionDetails
    let numberOfConfirmations: Int?

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Divider().padding(.vertical, 18)

            VStack(alignment: .leading, spacing: 8) {
                Text("Sent to")
                    .font(.footnote)
                    .foregroundColor(.secondary)
                    .multilineTextAlignment(.leading)

                Menu {
                    Button("Copy", systemImage: "doc.on.doc") {
                        UIPasteboard.general.string = transactionDetails.address().unformatted()
                    }
                } label: {
                    Text(transactionDetails.addressSpacedOut())
                        .multilineTextAlignment(.leading)
                }
                .fontWeight(.semibold)
                .font(.subheadline)
                .foregroundStyle(.primary)

                if transactionDetails.isConfirmed() {
                    HStack(spacing: 0) {
                        Group {
                            Text(transactionDetails.blockNumberFmt() ?? "")
                            Text("|")

                            if let numberOfConfirmations {
                                Group {
                                    Text(ThousandsFormatter(numberOfConfirmations).fmt())
                                        .contentTransition(.numericText())

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
                    .fontWeight(.bold)
                    .foregroundStyle(.tertiary.opacity(0.8))
                Spacer()

                VStack(alignment: .trailing) {
                    Text(transactionDetails.feeFmt(unit: metadata.selectedUnit) ?? "")
                    AsyncView(
                        cachedValue: transactionDetails.feeFiatFmtCached(),
                        operation: transactionDetails.feeFiatFmt
                    ) { amount in
                        Text(amount).foregroundStyle(.secondary)
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
                    AsyncView(
                        cachedValue: transactionDetails.sentSansFeeFiatFmtCached(),
                        operation: transactionDetails.sentSansFeeFiatFmt
                    ) { amount in
                        Text(amount).foregroundStyle(.secondary)
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
                    AsyncView(
                        cachedValue: transactionDetails.amountFiatFmtCached(),
                        operation: transactionDetails.amountFiatFmt
                    ) { amount in
                        Text(amount).foregroundStyle(.secondary)
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
