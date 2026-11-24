//
//  PayjoinSettingsScreen.swift
//  Cove
//

import SwiftUI

/// Settings > Payjoin, where the user picks how much of a Payjoin's extra fee this wallet offers
struct PayjoinSettingsScreen: View {
    @Binding var selection: PayjoinFeeHelp

    var body: some View {
        Form {
            Section("Fee help") {
                ForEach(PayjoinFeeHelp.allOptions, id: \.self) { option in
                    PayjoinFeeHelpRow(
                        option: option,
                        isSelected: selection == option,
                        select: { selection = option }
                    )
                }
            }
        }
        .scrollContentBackground(.hidden)
        .navigationTitle("Payjoin")
    }
}

private struct PayjoinFeeHelpRow: View {
    let option: PayjoinFeeHelp
    let isSelected: Bool
    let select: () -> Void

    var body: some View {
        Button(action: select) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text(option.title)
                        .font(.subheadline)

                    Text(option.explanation)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }

                Spacer()

                if isSelected {
                    Image(systemName: "checkmark")
                        .foregroundStyle(.blue)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
            }
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
        .accessibilityIdentifier(option.accessibilityIdentifier)
    }
}

private extension PayjoinFeeHelp {
    static let allOptions: [PayjoinFeeHelp] = [.upToOneInput, .dontHelp]

    var title: String {
        switch self {
        case .upToOneInput: "Up to one input (recommended)"
        case .dontHelp: "Don't help"
        }
    }

    var explanation: String {
        switch self {
        case .upToOneInput:
            "Lets the receiver use some of your change to pay for the coin they add, so the payment keeps your fee rate. Most wallets do this. The most it can cost is shown before you send."
        case .dontHelp:
            "You pay exactly the fee shown. Some receivers will decline, and the payment is sent as a normal transaction."
        }
    }

    var accessibilityIdentifier: String {
        switch self {
        case .upToOneInput: "payjoinSettings.feeHelp.upToOneInput"
        case .dontHelp: "payjoinSettings.feeHelp.dontHelp"
        }
    }
}

#Preview {
    NavigationStack {
        PayjoinSettingsScreen(selection: .constant(.upToOneInput))
    }
}
