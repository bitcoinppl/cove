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
    @State private var sheetIsOpen = false

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
            .onTapGesture { sheetIsOpen = true }

            Group {
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
                .font(.footnote)
                .fontWeight(.semibold)
            }
            .onTapGesture { toggleFiatOrBtc() }
        }
        .onChange(of: app.prices, initial: true) { _, newPrices in
            guard let prices = newPrices else { return }
            self.prices = prices
        }
        .sheet(isPresented: $sheetIsOpen) {
            MoreDetails(manager: manager, details: details, btcOrFiat: $btcOrFiat)
                .presentationDetents([.height(430), .height(550), .large])
                .padding(.horizontal)
        }
    }
}

extension SendFlowDetailsView {
    struct MoreDetails: View {
        @Environment(AppManager.self) private var app
        @Environment(\.dismiss) private var dismiss

        let manager: WalletManager
        let details: ConfirmDetails
        @Binding var btcOrFiat: FiatOrBtc

        // private
        @State private var splitOutput: SplitOutput? = nil

        func fiatAmount(_ amount: Amount) -> String {
            guard let prices = app.prices else {
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
                let units = manager.walletMetadata.selectedUnit == .sat ? "sats" : "btc"
                return "\(manager.amountFmt(amount)) \(units)"
            }
        }

        var body: some View {
            VStack(spacing: 32) {
                Text("More Details")
                    .fontWeight(.semibold)

                ScrollView {
                    VStack(spacing: 24) {
                        VStack(spacing: 10) {
                            HStack {
                                Text("Inputs")
                                    .font(.subheadline)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(.secondary)

                                Spacer()
                            }

                            VStack(spacing: 8) {
                                ForEach(details.inputs(), id: \.address) { input in
                                    HStack {
                                        Text(input.address.spacedOut())
                                            .fontWeight(.medium)
                                            .font(.caption)
                                            .foregroundStyle(.primary)
                                            .frame(maxWidth: screenWidth / 2, alignment: .leading)
                                            .multilineTextAlignment(.leading)

                                        Spacer()

                                        Text(displayFiatOrBtcAmount(input.amount))
                                            .font(.footnote)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                            }
                        }

                        if splitOutput == nil {
                            VStack(spacing: 10) {
                                HStack {
                                    Text("Outputs")
                                        .font(.subheadline)
                                        .fontWeight(.semibold)
                                        .foregroundStyle(.secondary)

                                    Spacer()
                                }

                                VStack(spacing: 8) {
                                    ForEach(details.outputs(), id: \.address) { output in
                                        HStack {
                                            Text(output.address.spacedOut())
                                                .fontWeight(.medium)
                                                .font(.caption)
                                                .foregroundStyle(.primary)
                                                .frame(maxWidth: screenWidth / 2, alignment: .leading)
                                                .multilineTextAlignment(.leading)

                                            Spacer()

                                            Text(displayFiatOrBtcAmount(output.amount))
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                }
                            }
                        }

                        if let splitOutput {
                            VStack(spacing: 10) {
                                HStack {
                                    Text("Outputs - External")
                                        .font(.subheadline)
                                        .fontWeight(.semibold)
                                        .foregroundStyle(.secondary)

                                    Spacer()
                                }

                                VStack(spacing: 8) {
                                    ForEach(splitOutput.external, id: \.address) { output in
                                        HStack {
                                            Text(output.address.spacedOut())
                                                .fontWeight(.medium)
                                                .font(.caption)
                                                .foregroundStyle(.primary)
                                                .frame(maxWidth: screenWidth / 2, alignment: .leading)
                                                .multilineTextAlignment(.leading)

                                            Spacer()

                                            Text(displayFiatOrBtcAmount(output.amount))
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                }
                            }

                            VStack(spacing: 10) {
                                HStack {
                                    Text("Outputs - Internal")
                                        .font(.subheadline)
                                        .fontWeight(.semibold)
                                        .foregroundStyle(.secondary)

                                    Spacer()
                                }

                                VStack(spacing: 8) {
                                    ForEach(splitOutput.internal, id: \.address) { output in
                                        HStack {
                                            Text(output.address.spacedOut())
                                                .fontWeight(.medium)
                                                .font(.caption)
                                                .foregroundStyle(.primary)
                                                .frame(maxWidth: screenWidth / 2, alignment: .leading)
                                                .multilineTextAlignment(.leading)

                                            Spacer()

                                            Text(displayFiatOrBtcAmount(output.amount))
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                }
                            }
                        }

                        VStack(spacing: 10) {
                            HStack {
                                Text("Fees")
                                    .font(.subheadline)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(.secondary)

                                Spacer()

                                Text(displayFiatOrBtcAmount(details.feeTotal()))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                } // </ScrollView>
                .onTapGesture {
                    btcOrFiat = btcOrFiat == .btc ? .fiat : .btc
                }

                Button(action: dismiss) {
                    Text("Close")
                        .frame(maxWidth: .infinity)
                        .padding()
                        .background(.midnightBlue)
                        .foregroundStyle(.white)
                        .cornerRadius(10)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
            }
            .padding()
            .task {
                splitOutput = try? await manager.rust.splitTransactionOutputs(outputs: details.outputs())
            }
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

#Preview("MoreDetails") {
    AsyncPreview {
        SendFlowDetailsView.MoreDetails(
            manager: WalletManager(preview: "preview_only"),
            details: ConfirmDetails.previewNew(),
            btcOrFiat: Binding.constant(.btc)
        )
        .environment(AppManager())
    }
}
