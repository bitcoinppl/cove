//
//  SendFlowDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import SwiftUI

struct SendFlowDetailsView: View {
    @Environment(AppManager.self) private var app

    // args
    let manager: WalletManager
    let details: ConfirmDetails
    @State var prices: PriceResponse?

    // private
    @State private var btcOrFiat = FiatOrBtc.btc

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    func fiatAmount(_ amount: Amount) -> String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        return manager.rust.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    func displayFiatOrBtcAmount(_ amount: Amount) -> String {
        switch btcOrFiat {
        case .fiat:
            return "≈ \(fiatAmount(amount))"
        case .btc:
            let units = metadata.selectedUnit == .sat ? "sats" : "btc"
            return "\(manager.amountFmt(amount)) \(units)"
        }
    }

    func toggleFiatOrBtc() {
        if prices == nil, app.prices == nil { return }
        let opposite = btcOrFiat == .btc ? FiatOrBtc.fiat : FiatOrBtc.btc
        btcOrFiat = opposite
    }

    var body: some View {
        VStack(spacing: 12) {
            // To Address Section
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

                Text(
                    details
                        .sendingTo()
                        .spacedOut()
                )
                .font(.system(.footnote, design: .none))
                .fontWeight(.semibold)
                .padding(.leading, 60)
                .lineLimit(3)
            }
            .padding(.top, 6)

            // Network Fee Section
            HStack {
                Text("Network Fee")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundStyle(.secondary)

                Spacer()

                Text(displayFiatOrBtcAmount(details.feeTotal()))
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 10)
            }

            // They receive section
            HStack {
                Text("They'll receive")
                Spacer()
                Text(displayFiatOrBtcAmount(details.sendingAmount()))
            }
            .font(.footnote)
            .fontWeight(.semibold)

            // Total Amount Section
            HStack {
                Text("You'll pay")
                Spacer()
                Text(displayFiatOrBtcAmount(details.spendingAmount()))
            }

            .onTapGesture { toggleFiatOrBtc() }
            .font(.footnote)
            .fontWeight(.semibold)
        }
        .onTapGesture { toggleFiatOrBtc() }
        .onChange(of: app.prices, initial: true) { _, newPrices in
            guard let prices = newPrices else { return }
            self.prices = prices
        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowDetailsView(
            manager: WalletManager(preview: "preview_only"),
            details: ConfirmDetails.previewNew(),
            prices: nil
        )
        .padding()
        .environment(AppManager())
    }
}
