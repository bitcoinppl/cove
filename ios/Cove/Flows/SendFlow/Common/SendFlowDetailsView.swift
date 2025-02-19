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

    // private
    @State private var presentingInputOutputDetails = false
    @State private var presentationSize: PresentationDetent = .medium

    var metadata: WalletMetadata { manager.walletMetadata }

    func fiatAmount(_ amount: Amount) -> String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        return manager.rust.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    func displayFiatOrBtcAmount(_ amount: Amount) -> String {
        switch metadata.fiatOrBtc {
        case .fiat:
            return "≈ \(fiatAmount(amount))"
        case .btc:
            let units = metadata.selectedUnit == .sat ? "sats" : "btc"
            return "\(manager.amountFmt(amount)) \(units)"
        }
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
            .onTapGesture { presentingInputOutputDetails = true }

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
            .onTapGesture { manager.dispatch(action: .toggleFiatOrBtc) }
        }
        .onChange(of: app.prices, initial: true) { _, newPrices in
            guard let prices = newPrices else { return }
            self.prices = prices
        }
        .sheet(isPresented: $presentingInputOutputDetails) {
            InputAndOutputDetailsView(manager: manager, details: details)
                .presentationDetents(
                    [.height(300), .height(400), .height(500), .large], selection: $presentationSize
                )
                .padding(.horizontal)
        }
        .onAppear {
            let total = details.outputs().count + details.inputs().count
            if total == 3 { presentationSize = .height(300) }
            if total > 3 { presentationSize = .height(400) }
            if total > 5 { presentationSize = .height(500) }
        }
    }
}

extension SendFlowDetailsView {
    struct InputAndOutputDetailsView: View {
        @Environment(AppManager.self) private var app
        @Environment(SendFlowPresenter.self) private var presenter
        @Environment(\.dismiss) private var dismiss

        let manager: WalletManager
        let details: ConfirmDetails

        // private
        @State private var splitOutput: SplitOutput? = nil

        var metadata: WalletMetadata { manager.walletMetadata }

        func fiatAmount(_ amount: Amount) -> String {
            guard let prices = app.prices else {
                app.dispatch(action: .updateFiatPrices)
                return "---"
            }

            return manager.rust.convertAndDisplayFiat(amount: amount, prices: prices)
        }

        func displayFiatOrBtcAmount(_ amount: Amount) -> String {
            switch metadata.fiatOrBtc {
            case .fiat:
                return "≈ \(fiatAmount(amount))"
            case .btc:
                let units = manager.walletMetadata.selectedUnit == .sat ? "sats" : "btc"
                return "\(manager.amountFmt(amount)) \(units)"
            }
        }

        var body: some View {
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
            .onTapGesture { manager.dispatch(action: .toggleFiatOrBtc) }
            .padding()
            .task {
                splitOutput = try? await manager.rust.splitTransactionOutputs(
                    outputs: details.outputs())
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
        .environment(AppManager.shared)
    }
}

#Preview("InputAndOutputDetailsView") {
    AsyncPreview {
        SendFlowDetailsView.InputAndOutputDetailsView(
            manager: WalletManager(preview: "preview_only"),
            details: ConfirmDetails.previewNew()
        )
        .environment(AppManager.shared)
        .environment(
            SendFlowPresenter(
                app: AppManager.shared, manager: WalletManager(preview: "preview_only")
            ))
    }
}
