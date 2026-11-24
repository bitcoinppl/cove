//
//  FeeBumpDetailsViews.swift
//  Cove
//
//  Created by Praveen Perera on 11/23/26.
//

import SwiftUI

extension TransactionDetails {
    /// A pending payment whose fee was bumped, which the header calls "Sped Up" until it
    /// confirms
    var isPendingFeeBump: Bool {
        !isConfirmed() && previousFee() != nil
    }
}

/// Where a pending send's details offer to bump its fee: a small button while it is only
/// pending, the "Taking longer than usual" banner once it is delayed
struct FeeBumpDetailsEntry: View {
    @Environment(\.openFeeBump) private var openFeeBump

    let transactionDetails: TransactionDetails
    let pendingSend: PendingSendStatus

    var body: some View {
        if pendingSend.isDelayed() {
            FeeBumpDelayedBanner(fee: transactionDetails.fee().map { "\($0.satsString()) sats" }, bump: bump)
                .padding(.horizontal, detailsExpandedPadding - 8)
                .padding(.top, 12)
        } else if pendingSend.isBumpable() {
            FeeBumpSmallButton(action: bump)
                .padding(.top, 10)
        }
    }

    private func bump() {
        openFeeBump(transactionDetails.txId())
    }
}

/// An understated Speed Up capsule for a send that isn't late yet
private struct FeeBumpSmallButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label("Speed Up", systemImage: "bolt.fill")
                .font(.caption)
                .fontWeight(.semibold)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .overlay(Capsule().stroke(Color.secondary.opacity(0.4), lineWidth: 1))
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("transactionDetails.speedUp")
    }
}

/// The orange "Taking longer than usual" banner with a prominent Speed Up button
private struct FeeBumpDelayedBanner: View {
    let fee: String?
    let bump: () -> Void

    private var message: String {
        guard let fee else { return "Your fee is below what the network is confirming right now." }
        return "Your fee (\(fee)) is below what the network is confirming right now."
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "exclamationmark")
                    .font(.caption.weight(.heavy))
                    .foregroundStyle(.white)
                    .frame(width: 22, height: 22)
                    .background(Circle().fill(Color.orange))

                VStack(alignment: .leading, spacing: 3) {
                    Text("Taking longer than usual")
                        .font(.subheadline)
                        .fontWeight(.semibold)

                    Text(message)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            Button(action: bump) {
                Label("Speed Up", systemImage: "bolt.fill")
            }
            .buttonStyle(DarkButtonStyle())
            .accessibilityIdentifier("transactionDetails.delayedBanner.speedUp")
        }
        .padding(16)
        .background(Color.orange.opacity(0.1))
        .overlay(RoundedRectangle(cornerRadius: 14).stroke(Color.orange.opacity(0.35), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 14))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("transactionDetails.delayedBanner")
    }
}

/// The fee a bumped payment paid just before its current one, struck through
struct SentPreviousFeeDetails: View {
    @Environment(AppManager.self) private var app

    let manager: WalletManager
    let previousFee: PreviousFee
    let unit: BitcoinUnit

    private var detail: String {
        let fiat = FeeBumpFiat(app: app, manager: manager).approximate(previousFee.fee)
        return [previousFee.feeRate.satsPerVbyteString(), fiat].compactMap(\.self).joined(separator: " · ")
    }

    var body: some View {
        HStack(alignment: .top) {
            Text("Previous Fee")
            Spacer()

            VStack(alignment: .trailing) {
                Text(previousFee.fee.fmtStringWithUnit(unit: unit))
                    .strikethrough()

                Text(detail)
                    .strikethrough()
                    .font(.caption)
                    .padding(.top, 2)
            }
        }
        .font(.subheadline)
        .foregroundStyle(.secondary)
        .padding(.bottom, 12)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("transactionDetails.previousFee")
    }
}

/// Every fee a payment paid as fee bumps replaced it, oldest first, collapsed until opened
struct SentFeeHistoryDetails: View {
    let transactionDetails: TransactionDetails
    let unit: BitcoinUnit

    @State private var isExpanded = false

    private struct Entry: Identifiable {
        let id: Int
        let title: String
        let fee: Amount
        let feeRate: FeeRate?
    }

    private var entries: [Entry] {
        let history = transactionDetails.feeHistory()
        var entries = history.enumerated().map { index, previous in
            Entry(
                id: index,
                title: index == 0 ? "Original" : "Bump \(index)",
                fee: previous.fee,
                feeRate: previous.feeRate
            )
        }

        if let fee = transactionDetails.fee() {
            entries.append(Entry(id: history.count, title: "Current", fee: fee, feeRate: transactionDetails.feeRate()))
        }

        return entries
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Divider().padding(.vertical, 18)

            Button(action: toggle) {
                HStack {
                    Text("Fee History")
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .accessibilityIdentifier("transactionDetails.feeHistory")
            .accessibilityValue(isExpanded ? "expanded" : "collapsed")

            if isExpanded {
                VStack(spacing: 10) {
                    ForEach(entries) { entry in
                        HStack {
                            Text(entry.title)
                            Spacer()
                            Text(entry.fee.fmtStringWithUnit(unit: unit))

                            if let feeRate = entry.feeRate {
                                Text("· \(feeRate.satsPerVbyteString())")
                                    .foregroundStyle(.tertiary)
                            }
                        }
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("transactionDetails.feeHistory.\(entry.id)")
                    }
                }
                .padding(.top, 12)
            }
        }
    }

    private func toggle() {
        withAnimation { isExpanded.toggle() }
    }
}

/// Why a pending bumped payment's ID differs from the one first sent
struct FeeBumpPendingFooterNote: View {
    var body: some View {
        Text("Speeding up replaces the original transaction, so the transaction ID changes.")
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.top, 18)
            .accessibilityIdentifier("transactionDetails.speedUpNote")
    }
}
