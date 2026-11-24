//
//  SendFlowDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import SwiftUI

struct SendFlowDetailsView: View {
    @Environment(AppManager.self) private var app
    @Environment(SendFlowPresenter.self) private var presenter

    // args
    let manager: WalletManager
    let details: ConfirmDetails
    @State var prices: PriceResponse?

    /// What a Payjoin payment offers toward the receiver's coin, nil when it offers nothing or is
    /// sent without Payjoin
    var payjoinFeeContribution: PayjoinFeeContribution? = nil

    // private
    @State private var presentingInputOutputDetails = false
    @State private var presentationSize: PresentationDetent = .medium

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    func fiatAmount(_ amount: Amount) -> String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        return manager.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    func displayFiatOrBtcAmount(_ amount: Amount) -> String {
        switch metadata.fiatOrBtc {
        case .fiat:
            "≈ \(fiatAmount(amount))"
        case .btc:
            manager.sendFlowAmountFmt(amount)
        }
    }

    /// The most the receiver can take from the change, under the fee
    private var payjoinFeeHelp: String? {
        guard let payjoinFeeContribution else { return nil }
        return "+ up to \(displayFiatOrBtcAmount(payjoinFeeContribution.max())) with Payjoin"
    }

    /// A Payjoin that offers fee help can cost up to its maximum, never less than the plain total
    private var spendingAmount: String {
        guard let payjoinFeeContribution else { return displayFiatOrBtcAmount(details.spendingAmount()) }
        return "up to \(displayFiatOrBtcAmount(payjoinFeeContribution.maxSpendingAmount()))"
    }

    var body: some View {
        VStack(spacing: 12) {
            SendFlowDetailsAddressRow(address: details.sendingTo())
                .padding(.top, 6)
                .onTapGesture { presentingInputOutputDetails = true }

            SendFlowDetailsAmountRows(
                feeAmount: displayFiatOrBtcAmount(details.feeTotal()),
                feePercentage: details.feePercentage(),
                payjoinFeeHelp: payjoinFeeHelp,
                receiveAmount: displayFiatOrBtcAmount(details.sendingAmount()),
                spendingAmount: spendingAmount
            )
            .onTapGesture { manager.dispatch(action: .toggleFiatOrBtc) }
        }
        .onChange(of: app.prices, initial: true) { _, newPrices in
            guard let prices = newPrices else { return }
            self.prices = prices
        }
        .sheet(isPresented: $presentingInputOutputDetails) {
            SendFlowAdvancedDetailsView(manager: manager, details: details)
                .presentationDetents(
                    [.height(300), .height(400), .height(500), .large], selection: $presentationSize
                )
        }
        .onAppear(perform: updatePresentationSize)
    }

    private func updatePresentationSize() {
        let total = details.outputs().count + details.inputs().count
        if total == 3 { presentationSize = .height(300) }
        if total > 3 { presentationSize = .height(400) }
        if total > 5 { presentationSize = .height(500) }
    }
}

private struct SendFlowDetailsAddressRow: View {
    let address: Address

    var body: some View {
        HStack {
            Text("Address")
                .font(.footnote)
                .fontWeight(.medium)
                .foregroundStyle(.secondary)
                .foregroundColor(.primary)

            Spacer()
            Spacer()
            Spacer()
            Spacer()

            Text(address: address)
                .font(.footnote)
                .fontWeight(.semibold)
                .padding(.leading, 60)
                .lineLimit(3)
        }
    }
}

private struct SendFlowDetailsAmountRows: View {
    let feeAmount: String
    let feePercentage: UInt64
    let payjoinFeeHelp: String?
    let receiveAmount: String
    let spendingAmount: String

    var body: some View {
        HStack {
            Text("Network Fee")
                .font(.footnote)
                .fontWeight(.medium)
                .foregroundStyle(.secondary)

            Spacer()

            VStack(alignment: .trailing, spacing: 2) {
                Text(feeAmount)
                    .font(.footnote)
                    .fontWeight(feePercentage > 20 ? .bold : .medium)
                    .foregroundStyle(feePercentage > 20 ? .red : .secondary)

                if let payjoinFeeHelp {
                    Text(payjoinFeeHelp)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("sendConfirm.payjoinFeeHelp")
                }
            }
            .padding(.vertical, 10)
        }

        HStack {
            Text("They'll receive")
            Spacer()
            Text(receiveAmount)
        }
        .font(.footnote)
        .fontWeight(.semibold)

        HStack {
            Text("You'll pay")
            Spacer()
            Text(spendingAmount)
                .accessibilityIdentifier("sendConfirm.totalSpending")
        }
        .font(.footnote)
        .fontWeight(.semibold)
    }
}

extension WalletManager {
    /// An amount in the wallet's bitcoin unit, labelled the way the send flow labels it
    func sendFlowAmountFmt(_ amount: Amount) -> String {
        let units = walletMetadata.selectedUnit == .sat ? "sats" : "btc"
        return "\(amountFmt(amount)) \(units)"
    }
}

#Preview {
    AsyncPreview {
        SendFlowDetailsView(
            manager: WalletManager(preview: .only),
            details: confirmDetailsPreviewNew(),
            prices: nil
        )
        .padding()
        .environment(AppManager.shared)
        .environment(
            SendFlowPresenter(
                app: AppManager.shared, manager: WalletManager(preview: .only)
            )
        )
    }
}
