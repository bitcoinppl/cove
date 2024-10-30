//
//  SendFlowSelectFeeRateView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import Foundation
import SwiftUI

private enum FeeSpeed {
    case fast
    case medium
    case slow

    var string: String {
        switch self {
        case .fast: "Fast"
        case .medium: "Medium"
        case .slow: "Slow"
        }
    }

    var circleColor: Color {
        switch self {
        case .fast:
            .green
        case .medium:
            .yellow
        case .slow:
            .orange
        }
    }
}

struct SendFlowSelectFeeRateView: View {
    var body: some View {
        VStack(spacing: 20) {
            Text("Network Fee")
                .font(.title3)
                .fontWeight(.bold)
                .padding(.bottom, 8)

            FeeOptionView(
                speed: .fast,
                duration: "30 minutes",
                satsPerVbyte: 4.46,
                sats: 620,
                fiatAmount: "≈ 0.36 USD",
                isSelected: false
            )

            FeeOptionView(
                speed: .medium,
                duration: "2 hours",
                satsPerVbyte: 2.48,
                sats: 250,
                fiatAmount: "≈ 0.16 USD",
                isSelected: true
            )

            FeeOptionView(
                speed: .slow,
                duration: "4 hours",
                satsPerVbyte: 1.24,
                sats: 120,
                fiatAmount: "≈ 0.06 USD",
                isSelected: false
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
    let speed: FeeSpeed
    let duration: String
    let satsPerVbyte: Double
    let sats: Int
    let fiatAmount: String
    let isSelected: Bool

    var fontColor: Color {
        if isSelected { .white } else { .primary }
    }

    var strokeColor: Color {
        if isSelected { Color.midnightBlue } else { Color(UIColor.systemGray2) }
    }

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(speed.string)
                        .font(.headline)
                        .foregroundColor(fontColor)

                    DurationCapsule(duration: duration, speed: speed, fontColor: fontColor)
                }
                Text("\(String(format: "%.2f", satsPerVbyte)) sats/vbyte")
                    .font(.subheadline)
                    .foregroundColor(fontColor)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 4) {
                Text("\(sats) sats")
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
    let duration: String
    let speed: FeeSpeed
    let fontColor: Color

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(speed.circleColor)
                .frame(width: 8, height: 8)
            Text(duration)
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
    SendFlowSelectFeeRateView()
}
