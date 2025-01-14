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
    let feeOptions: FeeRateOptionsWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee

    var body: some View {
        VStack(spacing: 20) {
            Text("Network Fee")
                .font(.title3)
                .fontWeight(.bold)
                .padding(.bottom, 8)

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
        }
        .padding(.horizontal)
        .padding(.top, 22)
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
        feeOption.satPerVb()
    }

    private var fiatAmount: String {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = feeOption.totalFee()
        return manager.rust.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(String(feeOption.feeSpeed()))
                        .font(.headline)
                        .foregroundColor(fontColor)

                    DurationCapsule(
                        speed: feeOption.feeSpeed(), fontColor: fontColor
                    )
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

private struct DurationCapsule: View {
    let speed: FeeSpeed
    let fontColor: Color

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(speed.circleColor)
                .frame(width: 8, height: 8)
            Text(speed.duration)
        }
        .font(.subheadline)
        .foregroundColor(fontColor)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Color.gray.opacity(0.2))
        .cornerRadius(8)
    }
}

#Preview {
    AsyncPreview {
        SendFlowSelectFeeRateView(
            manager: WalletManager(preview: "preview_only"),
            feeOptions: FeeRateOptionsWithTotalFee.previewNew(),
            selectedOption: Binding.constant(
                FeeRateOptionsWithTotalFee.previewNew().medium())
        )
        .environment(AppManager())
    }
}
