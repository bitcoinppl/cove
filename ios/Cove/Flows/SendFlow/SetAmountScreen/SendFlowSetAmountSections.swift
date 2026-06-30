//
//  SendFlowSetAmountSections.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/26.
//

import CoveCore
import SwiftUI

struct SendFlowAmountKeyboardToolbar: View {
    let addressIsEmpty: Bool
    let focusAddress: () -> Void
    let selectMax: () -> Void
    let clearAmount: () -> Void
    let dismissIfValid: () -> Void

    var body: some View {
        HStack {
            Group {
                if addressIsEmpty {
                    Button(action: focusAddress) {
                        Text("Next")
                    }
                } else {
                    Button(action: dismissIfValid) {
                        Text("Done")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Spacer()

            Button(action: selectMax) {
                Text("Max")
                    .font(.callout)
            }
            .tint(.primary)
            .buttonStyle(.bordered)

            Button(action: clearAmount) {
                Label("Clear", systemImage: "xmark.circle")
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Button(action: dismissIfValid) {
                Label("Done", systemImage: "keyboard.chevron.compact.down")
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.bordered)
            .tint(.primary)
        }
    }
}

struct SendFlowAddressKeyboardToolbar: View {
    let addressIsEmpty: Bool
    let addressIsValid: Bool
    let amountIsValid: Bool
    let pasteAddress: () -> Void
    let focusAmount: () -> Void
    let showQrScanner: () -> Void
    let clearAddress: () -> Void
    let dismissIfValid: () -> Void

    var body: some View {
        HStack {
            Group {
                if addressIsEmpty || !addressIsValid {
                    Button(action: pasteAddress) {
                        Text("Paste")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Group {
                if addressIsValid, !amountIsValid {
                    Button(action: focusAmount) {
                        Text("Next")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Button(action: showQrScanner) {
                Label("QR", systemImage: "qrcode")
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Spacer()

            Button(action: clearAddress) {
                Label("Clear", systemImage: "xmark.circle")
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Button(action: dismissIfValid) {
                Label("Done", systemImage: "keyboard.chevron.compact.down")
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.bordered)
            .tint(.primary)
        }
    }
}

struct SendFlowSetAmountToolbar: View {
    let focusField: SendFlowPresenter.FocusField?
    let addressIsEmpty: Bool
    let addressIsValid: Bool
    let amountIsValid: Bool
    let focusAddress: () -> Void
    let focusAmount: () -> Void
    let selectMax: () -> Void
    let clearAmount: () -> Void
    let pasteAddress: () -> Void
    let showQrScanner: () -> Void
    let clearAddress: () -> Void
    let dismissIfValid: () -> Void

    var body: some View {
        switch focusField {
        case .amount:
            SendFlowAmountKeyboardToolbar(
                addressIsEmpty: addressIsEmpty,
                focusAddress: focusAddress,
                selectMax: selectMax,
                clearAmount: clearAmount,
                dismissIfValid: dismissIfValid
            )
        case .address:
            SendFlowAddressKeyboardToolbar(
                addressIsEmpty: addressIsEmpty,
                addressIsValid: addressIsValid,
                amountIsValid: amountIsValid,
                pasteAddress: pasteAddress,
                focusAmount: focusAmount,
                showQrScanner: showQrScanner,
                clearAddress: clearAddress,
                dismissIfValid: dismissIfValid
            )
        case .none:
            EmptyView()
        }
    }
}

struct SendFlowCoinControlToolbar: View {
    let focusField: SendFlowPresenter.FocusField?
    let addressIsEmpty: Bool
    let addressIsValid: Bool
    let amountIsValid: Bool
    let pasteAddress: () -> Void
    let focusAmount: () -> Void
    let showQrScanner: () -> Void
    let clearAddress: () -> Void
    let dismissIfValid: () -> Void

    var body: some View {
        switch focusField {
        case .address:
            SendFlowAddressKeyboardToolbar(
                addressIsEmpty: addressIsEmpty,
                addressIsValid: addressIsValid,
                amountIsValid: amountIsValid,
                pasteAddress: pasteAddress,
                focusAmount: focusAmount,
                showQrScanner: showQrScanner,
                clearAddress: clearAddress,
                dismissIfValid: dismissIfValid
            )
        case .amount, .none:
            EmptyView()
        }
    }
}

struct SendFlowAmountInfoSection: View {
    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Text("Enter amount")
                    .font(.headline)
                    .fontWeight(.bold)

                Spacer()
            }
            .id(SendFlowPresenter.FocusField.amount)

            HStack {
                Text("How much would you like to send?")
                    .font(.footnote)
                    .foregroundStyle(.secondary.opacity(0.80))
                    .fontWeight(.medium)

                Spacer()
            }
        }
        .padding(.top)
    }
}

struct SendFlowCoinControlAmountSection: View {
    let totalSending: String
    let sendAmountFiat: String
    let unit: String
    let offset: CGFloat
    let canEditCustomAmount: Bool
    let showCustomAmount: () -> Void
    let updateUnit: (Unit) -> Void

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                Text(totalSending)
                    .font(.system(size: 48, weight: .bold))
                    .multilineTextAlignment(.center)
                    .keyboardType(.decimalPad)
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)
                    .scrollDisabled(true)
                    .offset(x: offset)
                    .padding(.horizontal, 30)
                    .frame(height: UIFont.boldSystemFont(ofSize: 48).lineHeight)
                    .onTapGesture {
                        guard canEditCustomAmount else { return }
                        showCustomAmount()
                    }

                HStack(spacing: 0) {
                    Menu {
                        VStack(alignment: .center, spacing: 0) {
                            Button(action: { updateUnit(.sat) }) {
                                Text("sats")
                                    .frame(maxWidth: .infinity)
                                    .padding(12)
                                    .background(Color.clear)
                            }
                            .buttonStyle(.plain)
                            .contentShape(Rectangle())

                            Button(action: { updateUnit(.btc) }) {
                                Text("btc")
                                    .frame(maxWidth: .infinity)
                                    .padding(12)
                                    .background(Color.clear)
                            }
                            .buttonStyle(.plain)
                            .contentShape(Rectangle())
                        }
                        .foregroundStyle(.primary.opacity(0.8))
                        .contentShape(Rectangle())
                    } label: {
                        HStack(spacing: 2) {
                            Text(unit)
                                .padding(.vertical, 10)
                                .padding(.horizontal, 10)
                                .fixedSize(horizontal: true, vertical: true)

                            Image(systemName: "chevron.down")
                                .font(.caption)
                                .fontWeight(.bold)
                                .padding(.top, 2)
                        }
                        .offset(y: -2)
                    }
                    .foregroundStyle(.primary)
                }
            }

            Text(sendAmountFiat)
                .contentTransition(.numericText())
                .font(.subheadline)
                .foregroundColor(.secondary)
        }
    }
}

struct SendFlowNetworkFeeSection: View {
    let selectedFeeRate: FeeRateOptionWithTotalFee?
    let totalFeeString: String?
    let showFeeSelection: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Network Fee")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fontWeight(.medium)

            HStack {
                Text(selectedFeeRate?.duration() ?? "")
                    .font(.caption2)
                    .foregroundStyle(.secondary)

                Button("Change speed", action: showFeeSelection)
                    .font(.caption2)
                    .foregroundColor(.blue)

                Spacer()

                AsyncText(text: totalFeeString, font: .footnote, color: .secondary, spinnerScale: 0.5)
            }
        }
        .onTapGesture(perform: showFeeSelection)
    }
}

struct SendFlowTotalSpendingSection: View {
    let totalSpentBtc: String
    let totalSpentInFiat: String

    var body: some View {
        VStack {
            HStack {
                Text("Total Spending")
                    .font(.footnote)
                    .fontWeight(.semibold)

                Spacer()

                Text(totalSpentBtc)
                    .multilineTextAlignment(.center)
                    .font(.footnote)
                    .fontWeight(.semibold)
            }

            HStack {
                Spacer()

                Text(totalSpentInFiat)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            .padding(.top, 1)
        }
    }
}

struct SendFlowCoinControlTotalSpendingSection: View {
    let utxoCount: Int
    let totalSpentBtc: String
    let totalSpentInFiat: String
    let showCustomAmount: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Total Spending")
                    .font(.footnote)
                    .fontWeight(.semibold)

                Spacer()

                Text(totalSpentBtc)
                    .multilineTextAlignment(.center)
                    .font(.footnote)
                    .fontWeight(.semibold)
            }

            HStack {
                Button(action: showCustomAmount) {
                    Text(utxoCount > 1 ? "Spending \(utxoCount) UTXOs" : "Spending 1 UTXO")
                        .font(.caption2)
                }
                .font(.caption2)
                .foregroundColor(.blue.opacity(0.8))

                Spacer()

                Text(totalSpentInFiat)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

struct SendFlowNextButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text("Next")
                .font(.footnote)
                .fontWeight(.semibold)
                .frame(maxWidth: .infinity)
                .padding()
                .background(Color.midnightBtn)
                .foregroundColor(.white)
                .cornerRadius(10)
        }
        .padding(.vertical, 10)
    }
}
