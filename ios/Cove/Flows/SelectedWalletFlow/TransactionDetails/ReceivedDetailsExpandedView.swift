//
//  ReceivedDetailsExpandedView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct ReceivedDetailsExpandedView: View {
    let manager: WalletManager
    let transactionDetails: TransactionDetails
    let numberOfConfirmations: Int?

    // private
    @State private var isCopied = false
    @State private var showingPriceInfo = false

    @ViewBuilder
    func expandedDetailsRow(header: String, content: String) -> some View {
        Text(header)
            .font(.caption)
            .foregroundColor(.gray)
            .multilineTextAlignment(.leading)

        Text(content)
            .fontWeight(.semibold)
            .multilineTextAlignment(.leading)
            .padding(.bottom, 14)
    }

    var body: some View {
        VStack(alignment: .leading) {
            Divider().padding(.vertical, 18)

            if transactionDetails.isConfirmed() {
                Text("Confirmations")
                    .font(.caption)
                    .foregroundColor(.gray)
                    .multilineTextAlignment(.leading)

                Group {
                    if let numberOfConfirmations {
                        Text(ThousandsFormatter(numberOfConfirmations).fmt())
                            .fontWeight(.semibold)
                            .multilineTextAlignment(.leading)
                            .contentTransition(.numericText())
                    } else {
                        ProgressView()
                            .tint(.primary)
                    }
                }
                .padding(.bottom, 14)

                expandedDetailsRow(
                    header: "Block Number",
                    content: String(transactionDetails.blockNumberFmt() ?? "")
                )
            }

            Text("Received At")
                .font(.caption)
                .foregroundColor(.gray)
                .multilineTextAlignment(.leading)

            HStack {
                Text(transactionDetails.addressSpacedOut())
                    .fontWeight(.semibold)
                    .multilineTextAlignment(.leading)
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
                    .foregroundColor(.primary)
                    .overlay(
                        RoundedRectangle(cornerRadius: 20)
                            .stroke(Color.gray.opacity(0.3), lineWidth: 1)
                    )
                    .frame(minWidth: 100)
                }
                .buttonStyle(PlainButtonStyle())
            }

            // MARK: - Fiat Price Section

            if transactionDetails.isConfirmed() {
                Divider().padding(.vertical, 18)

                HStack(alignment: .top) {
                    Text("Fiat Price")
                    Spacer()

                    VStack(alignment: .trailing, spacing: 4) {
                        // current fiat value
                        AsyncView(
                            cachedValue: transactionDetails.amountFiatFmtCached(),
                            operation: transactionDetails.amountFiatFmt
                        ) { amount in
                            Text(amount)
                                .font(.subheadline)
                                .fontWeight(.semibold)
                                .foregroundStyle(.primary)
                        }

                        // historical fiat value (when received)
                        HStack(spacing: 4) {
                            Image(systemName: "clock")
                                .font(.caption2)
                            AsyncView(
                                cachedValue: transactionDetails.historicalFiatFmtCached(),
                                operation: transactionDetails.historicalFiatFmt
                            ) { amount in
                                Text(amount)
                                    .font(.caption)
                                    .foregroundStyle(.primary)
                            }
                            Image(systemName: "info.circle")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                                .onTapGesture { showingPriceInfo.toggle() }
                                .popover(isPresented: $showingPriceInfo) {
                                    Text("Price at time of transaction")
                                        .font(.caption)
                                        .padding(8)
                                        .presentationCompactAdaptation(.popover)
                                }
                        }
                        .foregroundStyle(.primary)
                    }
                }
                .font(.subheadline)
                .foregroundStyle(.primary)
            }
        }
        .padding(.horizontal, detailsExpandedPadding)
    }
}
