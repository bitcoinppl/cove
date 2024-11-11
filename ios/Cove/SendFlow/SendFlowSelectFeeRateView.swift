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
    let txnSize: Int

    @State var selectedOption: FeeSpeed = .medium

    var body: some View {
        VStack(spacing: 20) {
            Text("Network Fee")
                .font(.title3)
                .fontWeight(.bold)
                .padding(.bottom, 8)

            FeeOptionView(
                feeOption: feeOptions.fast(),
                fiatAmount: "",
                txnSize: txnSize,
                isSelected: selectedOption == .fast
            )

            FeeOptionView(
                feeOption: feeOptions.medium(), fiatAmount: "", txnSize: txnSize, isSelected: selectedOption == .medium
            )

            FeeOptionView(
                feeOption: feeOptions.slow(), fiatAmount: "",
                txnSize: txnSize, isSelected: selectedOption == .slow
            )

            Button(action: {
                // Handle learn more action
            }) {
                Text("Learn More")
                    .foregroundColor(.white)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(Color.midnightBlue)
                    .cornerRadius(10)
            }
            .padding(.top, 20)
            .padding(.horizontal, 32)
        }
        .padding()
    }
}

private struct FeeOptionView: View {
    let feeOption: FeeRateOption
    let fiatAmount: String
    let txnSize: Int

    let isSelected: Bool

    var fontColor: Color {
        if isSelected { .white } else { .primary }
    }

    var strokeColor: Color {
        if isSelected { Color.midnightBlue } else { Color(UIColor.systemGray2) }
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
                    Text(feeOption.duration())
                        .font(.headline)
                        .foregroundColor(fontColor)

                    DurationCapsule(
                        speed: feeOption.feeSpeed(), fontColor: fontColor
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
    SendFlowSelectFeeRateView(feeOptions: FeeRateOptions.preview_new(), txnSize: 3040)
}
