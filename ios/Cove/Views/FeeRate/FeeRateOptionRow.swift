//
//  FeeRateOptionRow.swift
//  Cove
//
//  Created by Praveen Perera on 11/23/26.
//

import SwiftUI

/// One fee rate a transaction can pay: its speed, rate, total fee and that fee in fiat
///
/// Fed by plain data, so the send flow's fee picker and the bump fee sheet draw the same row
struct FeeRateOptionRow: View {
    @Environment(\.colorScheme) private var colorScheme

    let feeOption: FeeRateOptionWithTotalFee
    let isSelected: Bool

    /// The total fee in fiat, such as "≈ 1.07 USD", `nil` while it loads
    let fiatAmount: String?

    /// Shown in place of the total when the option has none, `nil` shows a spinner instead
    var missingTotalText: String? = nil

    /// Prefix of the row's accessibility identifier, followed by its speed
    let identifierPrefix: String
    let onSelect: () -> Void

    private var fontColor: Color {
        isSelected ? .white : .primary
    }

    private var strokeColor: Color {
        if colorScheme == .dark {
            return isSelected ? Color.primary.opacity(0.75) : Color(UIColor.tertiaryLabel).opacity(0.6)
        }

        return isSelected ? Color.primary : Color.secondary
    }

    private var totalFeeText: String? {
        guard let totalFee = feeOption.totalFee() else { return missingTotalText }
        return "\(totalFee.satsString()) sats"
    }

    private var fiatText: String? {
        guard feeOption.totalFee() != nil else { return missingTotalText.map { _ in "" } }
        return fiatAmount
    }

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(String(feeOption.feeSpeed()))
                        .font(.headline)
                        .foregroundColor(fontColor)

                    SendFlowDurationCapsule(speed: feeOption.feeSpeed(), fontColor: fontColor)
                        .font(.caption2)
                }

                Text(feeOption.satsPerVbyteString())
                    .font(.subheadline)
                    .foregroundColor(fontColor)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 4) {
                AsyncText(text: totalFeeText, font: .headline, color: fontColor)
                AsyncText(text: fiatText, font: .subheadline, color: fontColor, spinnerScale: 0.8)
            }
        }
        .padding()
        .background(isSelected ? Color.midnightBtn.opacity(0.8) : Color(UIColor.systemGray6))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(strokeColor, lineWidth: 1))
        .cornerRadius(12)
        .contentShape(Rectangle())
        .onTapGesture(perform: onSelect)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("\(identifierPrefix).\(feeOption.feeSpeed().identifierKey)")
    }
}

extension FeeSpeed {
    /// Names the speed in accessibility identifiers, which a custom rate's duration would make
    /// unpredictable
    var identifierKey: String {
        switch self {
        case .fast: "fast"
        case .medium: "medium"
        case .slow: "slow"
        case .custom: "custom"
        }
    }
}
