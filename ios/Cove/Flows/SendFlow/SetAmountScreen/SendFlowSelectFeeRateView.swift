//
//  SendFlowSelectFeeRateView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import Foundation
import SwiftUI

struct SendFlowSelectFeeRateView: View {
    let manager: WalletManager

    @Binding var feeOptions: FeeRateOptionsWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee
    @Binding var selectedPresentationDetent: PresentationDetent

    // private
    @State private var isPresentingCustomFeeSelection: Bool = false

    @ViewBuilder
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

private struct FeeOptionView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme

    // passed in args
    let manager: WalletManager
    let feeOption: FeeRateOptionWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee

    var isSelected: Bool {
        selectedOption.feeSpeed() == feeOption.feeSpeed()
    }

    var fontColor: Color {
        if isSelected { .white } else { .primary }
    }

    var strokeColor: Color {
        if colorScheme == .dark {
            return if isSelected { Color.primary.opacity(0.75) } else {
                Color(UIColor.tertiaryLabel).opacity(0.6)
            }
        }

        // light
        return if isSelected { Color.primary } else { Color.secondary }
    }

    var isLoading: Bool {
        feeOption.totalFee() == nil
    }

    var totalFee: String? {
        feeOption.totalFee()?.satsString()
    }

    var satsPerVbyte: Double {
        Double(feeOption.satPerVb())
    }

    private var fiatAmount: String? {
        guard let totalFee = feeOption.totalFee() else { return nil }

        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        return "≈ \(manager.rust.convertAndDisplayFiat(amount: totalFee, prices: prices))"
    }

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(String(feeOption.feeSpeed()))
                        .font(.headline)
                        .foregroundColor(fontColor)

                    SendFlowDurationCapsule(
                        speed: feeOption.feeSpeed(), fontColor: fontColor
                    )
                    .font(.caption2)
                }

                HStack {
                    Text("\(String(format: "%.2f", satsPerVbyte)) sats/vbyte")
                        .font(.subheadline)
                        .foregroundColor(fontColor)
                }
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 4) {
                AsyncText(
                    text: totalFee.map { "\($0) sats" },
                    font: .headline,
                    color: fontColor
                )

                AsyncText(
                    text: fiatAmount,
                    font: .subheadline,
                    color: fontColor,
                    spinnerScale: 0.8
                )
            }
        }
        .padding()
        .background(
            isSelected
                ? Color.midnightBtn.opacity(0.8) : Color(UIColor.systemGray6)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(strokeColor, lineWidth: 1)
        )
        .onTapGesture {
            selectedOption = feeOption
            dismiss()
        }
        .cornerRadius(12)
    }
}

#Preview("Select Fee Rate") {
    AsyncPreview {
        VStack {
            SendFlowSelectFeeRateView(
                manager: WalletManager(preview: "preview_only"),
                feeOptions: Binding.constant(FeeRateOptionsWithTotalFee.previewNew()),
                selectedOption: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().medium()
                ),
                selectedPresentationDetent: Binding.constant(PresentationDetent.large)
            )
            .environment(WalletManager(preview: "preview_only"))
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
                manager: WalletManager(preview: "preview_only"),
                feeOptions: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().addCustomFeeRate(
                        feeRate: FeeRateOptionWithTotalFee(
                            feeSpeed: FeeSpeed.custom(durationMins: 10),
                            feeRate: FeeRate.fromSatPerVb(satPerVb: 143.00),
                            totalFee: Amount.fromSat(sats: 3000)
                        ))),
                selectedOption: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().medium()
                ),
                selectedPresentationDetent: Binding.constant(PresentationDetent.large)
            )
            .environment(WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
            .frame(height: 550)
        }
        .frame(maxHeight: .infinity)
        .background(.coveBg)
    }
}
