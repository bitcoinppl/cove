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
    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
    let showLockStateUpdatingIndicator: Bool
    let lockStateLoadError: String?
    let retryLockState: () -> Void
    let toggleLockState: () -> Void

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Divider().padding(.vertical, 18)

            SentTransactionDestination(
                manager: manager,
                transactionDetails: transactionDetails,
                numberOfConfirmations: numberOfConfirmations,
                lockState: lockState,
                isUpdatingLockState: isUpdatingLockState,
                showLockStateUpdatingIndicator: showLockStateUpdatingIndicator,
                lockStateLoadError: lockStateLoadError,
                retryLockState: retryLockState,
                toggleLockState: toggleLockState
            )

            SentFiatPriceDetails(transactionDetails: transactionDetails)

            Divider().padding(.vertical, 18)

            SentNetworkFeeDetails(
                transactionDetails: transactionDetails,
                unit: metadata.selectedUnit
            )
            SentRecipientAmountDetails(
                transactionDetails: transactionDetails,
                unit: metadata.selectedUnit
            )

            Divider().padding(.vertical, 18)

            SentTotalDetails(
                transactionDetails: transactionDetails,
                unit: metadata.selectedUnit
            )
        }
        .padding(.horizontal, detailsExpandedPadding)
    }
}

private struct SentTransactionDestination: View {
    let manager: WalletManager
    let transactionDetails: TransactionDetails
    let numberOfConfirmations: Int?
    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
    let showLockStateUpdatingIndicator: Bool
    let lockStateLoadError: String?
    let retryLockState: () -> Void
    let toggleLockState: () -> Void

    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Sent to")
                    .font(.footnote)
                    .foregroundColor(.secondary)
                    .multilineTextAlignment(.leading)

                Menu {
                    if transactionDetails.address() != nil {
                        Button("Copy", systemImage: "doc.on.doc", action: copyAddress)
                    }
                } label: {
                    Text(transactionDetails.addressSpacedOut() ?? "Address unavailable")
                        .multilineTextAlignment(.leading)
                }
                .fontWeight(.semibold)
                .font(.subheadline)
                .foregroundStyle(.primary)

                SentConfirmationMetadata(
                    manager: manager,
                    transactionDetails: transactionDetails,
                    numberOfConfirmations: numberOfConfirmations
                )
            }

            Spacer()

            TransactionDetailsLockControl(
                lockState: lockState,
                isUpdatingLockState: isUpdatingLockState,
                showLockStateUpdatingIndicator: showLockStateUpdatingIndicator,
                lockStateLoadError: lockStateLoadError,
                retryLockState: retryLockState,
                toggleLockState: toggleLockState
            )
            .padding(.top, 1)
        }
    }

    private func copyAddress() {
        guard let address = transactionDetails.address() else { return }

        UIPasteboard.general.string = address.unformatted()
    }
}

private struct SentConfirmationMetadata: View {
    let manager: WalletManager
    let transactionDetails: TransactionDetails
    let numberOfConfirmations: Int?

    var body: some View {
        if transactionDetails.isConfirmed() {
            HStack(spacing: 0) {
                Group {
                    Text(transactionDetails.blockNumberFmt() ?? "")
                    Text("|")

                    if let numberOfConfirmations {
                        Text(manager.displayConfirmationCount(UInt32(numberOfConfirmations)))
                            .contentTransition(.numericText())

                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 10))
                            .fontWeight(.bold)
                            .foregroundStyle(.green)
                            .padding(.leading, 3)
                    }
                }
                .padding(.horizontal, 2)
            }
            .font(.caption)
            .foregroundStyle(.tertiary)
        }
    }
}

private struct SentFiatPriceDetails: View {
    let transactionDetails: TransactionDetails

    var body: some View {
        if transactionDetails.isConfirmed() {
            VStack(alignment: .leading, spacing: 0) {
                Divider().padding(.vertical, 18)

                HStack(alignment: .top) {
                    Text("Fiat Price")
                    Spacer()

                    SentFiatPriceValues(transactionDetails: transactionDetails)
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)
            }
        }
    }
}

private struct SentFiatPriceValues: View {
    let transactionDetails: TransactionDetails

    @State private var showingPriceInfo = false

    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
            AsyncView(
                cachedValue: transactionDetails.amountFiatFmtCached(),
                operation: transactionDetails.amountFiatFmt
            ) { amount in
                Text(amount)
                    .font(.subheadline)
                    .fontWeight(.semibold)
            }

            SentHistoricalFiatPrice(
                transactionDetails: transactionDetails,
                showingPriceInfo: $showingPriceInfo
            )
        }
    }
}

private struct SentHistoricalFiatPrice: View {
    let transactionDetails: TransactionDetails
    @Binding var showingPriceInfo: Bool

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "clock")
                .font(.caption2)
            AsyncView(
                cachedValue: transactionDetails.historicalFiatFmtCached(),
                operation: transactionDetails.historicalFiatFmt
            ) { amount in
                Text(amount)
                    .font(.caption)
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
        .foregroundStyle(.secondary)
    }
}

private struct SentNetworkFeeDetails: View {
    let transactionDetails: TransactionDetails
    let unit: BitcoinUnit

    var body: some View {
        HStack(alignment: .top) {
            Text("Network Fee")
            Image(systemName: "info.circle")
                .font(.footnote)
                .fontWeight(.bold)
                .foregroundStyle(.tertiary.opacity(0.8))
            Spacer()

            VStack(alignment: .trailing) {
                Text(transactionDetails.feeFmt(unit: unit) ?? "")
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
    }
}

private struct SentRecipientAmountDetails: View {
    let transactionDetails: TransactionDetails
    let unit: BitcoinUnit

    var body: some View {
        HStack(alignment: .top) {
            Text("Receipient Receives")
            Spacer()

            VStack(alignment: .trailing) {
                Text(transactionDetails.sentSansFeeFmt(unit: unit) ?? "")
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
    }
}

private struct SentTotalDetails: View {
    let transactionDetails: TransactionDetails
    let unit: BitcoinUnit

    var body: some View {
        HStack(alignment: .top) {
            Text("Total Spent")
            Spacer()

            VStack(alignment: .trailing) {
                Text(transactionDetails.amountFmt(unit: unit))
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
}
