//
//  SendFlowSelectFeeRateView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import Foundation
import SwiftUI

struct SendFlowSelectFeeRateView: View {
    enum Screen { case select, custom }

    let manager: WalletManager

    @Binding var feeOptions: FeeRateOptionsWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee
    @Binding var selectedPresentationDetent: PresentationDetent

    // private
    @State private var route: [Screen] = []

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

            Button(action: {
                route = [.custom]
            }) {
                Text("Customize Fee")
                    .font(.subheadline)
                    .fontWeight(.semibold)
            }
            .frame(maxWidth: .infinity)
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
        NavigationStack(path: $route) {
            SelectView
                .navigationDestination(
                    for: Screen.self,
                    destination: { route in
                        switch route {
                        case .custom: SendFlowCustomFeeRateView(
                                feeOptions: $feeOptions,
                                selectedOption: $selectedOption,
                                selectedPresentationDetent: $selectedPresentationDetent
                            )
                        case .select: SelectView
                        }
                    }
                )
        }
    }
}

private struct FeeOptionView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss

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
        if isSelected { Color.midnightBtn } else { Color.secondary }
    }

    var totalFee: String {
        feeOption.totalFee().satsString()
    }

    var satsPerVbyte: Double {
        Double(feeOption.satPerVb())
    }

    private var fiatAmount: String {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = feeOption.totalFee()
        return "≈ \(manager.rust.convertAndDisplayFiat(amount: amount, prices: prices))"
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
                Text("\(totalFee) sats")
                    .font(.headline)
                    .foregroundColor(fontColor)

                Text(fiatAmount)
                    .font(.subheadline)
                    .foregroundColor(fontColor)
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
            .environment(AppManager())
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
                feeOptions: Binding.constant(FeeRateOptionsWithTotalFee.previewNew().addCustomFee(feeRate: 13.77)),
                selectedOption: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().medium()
                ),
                selectedPresentationDetent: Binding.constant(PresentationDetent.large)
            )
            .environment(WalletManager(preview: "preview_only"))
            .environment(AppManager())
            .frame(height: 550)
        }
        .frame(maxHeight: .infinity)
        .background(.coveBg)
    }
}
