//
//  SendFlowSelectFeeRateView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import Foundation
import SwiftUI

struct SendFlowSelectFeeRateView: View {
    let feeOptions: FeeRateOptions
    let txnSize: Double

    @Binding var selectedOption: FeeRateOption

    var body: some View {
        VStack(spacing: 20) {
            Text("Network Fee")
                .font(.title3)
                .fontWeight(.bold)
                .padding(.bottom, 8)

            FeeOptionView(
                feeOption: feeOptions.fast(),
                fiatAmount: "",
                selectedOption: $selectedOption
            )

            FeeOptionView(
                feeOption: feeOptions.medium(),
                fiatAmount: "",
                selectedOption: $selectedOption
            )

            FeeOptionView(
                feeOption: feeOptions.slow(),
                fiatAmount: "",
                selectedOption: $selectedOption
            )
        }
        .padding(.horizontal)
        .padding(.top, 22)
    }
}

private struct FeeOptionView: View {
    @Environment(\.dismiss) private var dismiss

    let feeOption: FeeRateOption
    let fiatAmount: String

    @Binding var selectedOption: FeeRateOption

    var isSelected: Bool {
        selectedOption.speed() == feeOption.speed()
    }

    var fontColor: Color {
        if isSelected { .white } else { .primary }
    }

    var strokeColor: Color {
        if isSelected { Color.midnightBlue } else { Color.secondary }
    }

    var totalFee: String {
        feeOption.totalFee(txnSize: UInt64(txnSize)).map { $0.satsString() } ?? "---"
    }

    var satsPerVbyte: Double {
        feeOption.satPerVb()
    }

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(String(feeOption.speed()))
                        .font(.headline)
                        .foregroundColor(fontColor)

                    DurationCapsule(
                        speed: feeOption.speed(), fontColor: fontColor
                    )
                }
                Text("\(String(format: "%.2f", satsPerVbyte)) sats/vbyte")
                    .font(.subheadline)
                    .foregroundColor(fontColor)
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
        .background(isSelected ? Color.midnightBlue.opacity(0.8) : Color(UIColor.systemGray6))
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
    SendFlowSelectFeeRateView(
        feeOptions: FeeRateOptions.previewNew(),
        txnSize: 3040,
        selectedOption: Binding.constant(FeeRateOptions.previewNew().medium())
    )
}
