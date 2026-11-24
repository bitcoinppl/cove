//
//  SendFlowSelectFeeRateView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import SwiftUI

struct SendFlowSelectFeeRateView: View {
    let manager: WalletManager

    @Binding var feeOptions: FeeRateOptionsWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee
    @Binding var selectedPresentationDetent: PresentationDetent

    /// private
    @State private var isPresentingCustomFeeSelection: Bool = false

    var SelectView: some View {
        VStack(spacing: 20) {
            Text("Network Fee")
                .font(.title3)
                .fontWeight(.bold)
                .padding(.vertical, 8)

            FeeOptionView(
                manager: manager,
                feeOption: feeOptions.fast(),
                selectedOption: $selectedOption
            )

            FeeOptionView(
                manager: manager,
                feeOption: feeOptions.medium(),
                selectedOption: $selectedOption
            )

            FeeOptionView(
                manager: manager,
                feeOption: feeOptions.slow(),
                selectedOption: $selectedOption
            )

            if let custom = feeOptions.custom() {
                FeeOptionView(
                    manager: manager,
                    feeOption: custom,
                    selectedOption: $selectedOption
                )
            }

            Button(action: { isPresentingCustomFeeSelection = true }) {
                Text("Customize Fee")
                    .font(.subheadline)
                    .fontWeight(.semibold)
                    .frame(maxWidth: .infinity)
            }
            .padding()
            .background(Color.midnightBtn)
            .foregroundColor(.white)
            .cornerRadius(10)
            .padding(.horizontal, detailsExpandedPadding)
            .padding(.vertical, 12)
        }
        .padding(.horizontal)
        .padding(.top, 22)
    }

    var body: some View {
        SelectView
            .sheet(isPresented: $isPresentingCustomFeeSelection) {
                SendFlowCustomFeeRateView(
                    feeOptions: $feeOptions,
                    selectedOption: $selectedOption,
                    selectedPresentationDetent: $selectedPresentationDetent
                )
                .presentationDetents([.height(350)])
            }
    }
}

/// A fee option in the send flow's picker, which picks it and closes the sheet on tap
private struct FeeOptionView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss

    // passed in args
    let manager: WalletManager
    let feeOption: FeeRateOptionWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee

    private var fiatAmount: String? {
        guard let totalFee = feeOption.totalFee() else { return nil }

        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        return "≈ \(manager.convertAndDisplayFiat(amount: totalFee, prices: prices))"
    }

    var body: some View {
        FeeRateOptionRow(
            feeOption: feeOption,
            isSelected: selectedOption.feeSpeed() == feeOption.feeSpeed(),
            fiatAmount: fiatAmount,
            identifierPrefix: "sendFlow.feeOption",
            onSelect: select
        )
    }

    private func select() {
        selectedOption = feeOption
        dismiss()
    }
}

#Preview("Select Fee Rate") {
    AsyncPreview {
        VStack {
            SendFlowSelectFeeRateView(
                manager: WalletManager(preview: .only),
                feeOptions: Binding.constant(FeeRateOptionsWithTotalFee.previewNew()),
                selectedOption: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().medium()
                ),
                selectedPresentationDetent: Binding.constant(PresentationDetent.large)
            )
            .environment(WalletManager(preview: .only))
            .environment(AppManager.shared)
            .frame(height: 440)
        }
        .frame(maxHeight: .infinity)
        .background(.coveBg)
    }
}

#Preview("Select Fee Rate with Custom") {
    AsyncPreview {
        VStack {
            SendFlowSelectFeeRateView(
                manager: WalletManager(preview: .only),
                feeOptions: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().addCustomFeeRate(
                        feeRate: FeeRateOptionWithTotalFee(
                            feeSpeed: FeeSpeed.custom(durationMins: 10),
                            feeRate: FeeRate.fromSatPerVb(satPerVb: 143.00),
                            totalFee: Amount.fromSat(sats: 3000)
                        )
                    )
                ),
                selectedOption: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().medium()
                ),
                selectedPresentationDetent: Binding.constant(PresentationDetent.large)
            )
            .environment(WalletManager(preview: .only))
            .environment(AppManager.shared)
            .frame(height: 550)
        }
        .frame(maxHeight: .infinity)
        .background(.coveBg)
    }
}
