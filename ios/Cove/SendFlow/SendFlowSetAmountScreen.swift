//
//  SendFlowSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

private enum FocusField: Hashable {
    case amount
    case address
}

struct SendFlowSetAmountScreen: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId
    @State var model: WalletViewModel
    @State var address: String = ""

    // private
    @FocusState private var focusField: FocusField?
    @State private var scrollPosition: ScrollPosition = .init(idType: FocusField.self)

    // text inputs
    @State private var sendAmount: String = "0"
    @State private var sendAmountFiat: String = "≈ $0.00"

    private var metadata: WalletMetadata {
        model.walletMetadata
    }

    private var formatter: NumberFormatter {
        let f = NumberFormatter()
        f.numberStyle = .currency

        if metadata.selectedUnit == .btc {
            f.minimumFractionDigits = 0
            f.maximumFractionDigits = 0
        } else {
            f.minimumFractionDigits = 2
            f.maximumFractionDigits = 2
        }
        return f
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(model: model, amount: model.balance.confirmed)

            // MARK: CONTENT

            ScrollView {
                VStack(spacing: 24) {
                    // Set amount, header and text
                    AmountInfoSection

                    // Amount input
                    EnterAmountSection

                    // Address Section
                    EnterAddressSection

                    // Account Section
                    AccountSection

                    // Network Fee Section
                    NetworkFeeSection

                    // Total Section
                    TotalSection

                    Spacer()

                    // Next Button
                    NextButtonBottom
                }
            }
            .padding(.horizontal)
            .frame(width: screenWidth)
            .background(colorScheme == .light ? .white : .black)
            .scrollIndicators(.hidden)
            .scrollPosition($scrollPosition, anchor: .top)
        }
        .padding(.top, 0)
        .navigationTitle("Send")
        .navigationBarTitleDisplayMode(.inline)
        .onChange(of: focusField, initial: false, focusFieldChanged)
        .toolbar {
            ToolbarItemGroup(placement: .keyboard) {
                ToolBarView
            }
        }
    }

    private func amountSats(_ amount: Double) -> Double {
        if amount == 0 {
            return 0
        }

        if metadata.selectedUnit == .sat {
            return amount
        }

        return amount * 100_000_000
    }

    private func sendAmountChanged(_ oldValue: String, _ value: String) {
        // allow clearing completely
        if value == "" {
            sendAmountFiat = "≈ $0.00"
            return
        }

        let value = value.removingLeadingZeros()
        sendAmount = value

        guard let amount = Double(value) else {
            sendAmount = oldValue
            return
        }

        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            sendAmountFiat = "---"
            return
        }

        let amountSats = amountSats(amount)
        let fiatAmount = (amountSats / 100_000_000) * Double(prices.usd)

        sendAmountFiat = "≈ \(formatter.string(from: NSNumber(value: fiatAmount)) ?? "$0.00")"
    }

    private func selectedUnitChanged(_ oldUnit: Unit, _ newUnit: Unit) {
        guard let amount = Double(sendAmount) else { return }
        if amount == 0 { return }
        if oldUnit == newUnit { return }

        switch newUnit {
        case .btc:
            sendAmount = String(amount / 100_000_000)
        case .sat:
            sendAmount = String(Int(amount * 100_000_000))
        }
    }

    private func focusFieldChanged(_ oldField: FocusField?, _ newField: FocusField?) {
        print("focusFieldChangedl \(oldField) --> \(newField)")

        DispatchQueue.main.async {
            withAnimation(.easeInOut(duration: 0.3)) {
                scrollPosition.scrollTo(id: newField)
            }
        }
    }

    @ViewBuilder
    var AmountKeyboardToolbar: some View {
        HStack {
            Button(action: { focusField = .address }) {
                Text("Enter Address")
            }
            .tint(Color.midnightBlue)
            .buttonStyle(.bordered)

            Spacer()

            Button(action: {
                // TODO: add  max
            }) {
                Text("Max")
            }
            .tint(.green)
            .buttonStyle(.bordered)

            Button(action: { sendAmount = "" }) {
                Label("Clear", systemImage: "xmark.circle.fill")
            }
            .buttonStyle(.bordered)
            .tint(.red)

            Button(action: { focusField = .none }) {
                Label("Done", systemImage: "keyboard.chevron.compact.down")
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.bordered)
            .tint(colorScheme == .light ? .black : .white)
        }
    }

    @ViewBuilder
    var AddressKeyboardToolbar: some View {
        HStack {
            Button(action: { focusField = .amount }) {
                Text("Enter Amount")
            }
            .tint(Color.midnightBlue)
            .buttonStyle(.bordered)

            Spacer()

            Button(action: { address = "" }) {
                Label("Clear", systemImage: "xmark.circle.fill")
            }
            .buttonStyle(.bordered)
            .tint(.red)

            Button(action: { focusField = .none }) {
                Label("Done", systemImage: "keyboard.chevron.compact.down")
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.bordered)
            .tint(colorScheme == .light ? .black : .white)
        }
    }

    @ViewBuilder
    var ToolBarView: some View {
        switch focusField {
        case .amount: AmountKeyboardToolbar
        case .address: AddressKeyboardToolbar
        case .none: EmptyView()
        }
    }

    @ViewBuilder
    var AmountInfoSection: some View {
        VStack(spacing: 8) {
            HStack {
                Text("Set amount")
                    .font(.title3)
                    .fontWeight(.bold)

                Spacer()
            }
            .id(FocusField.amount)
            .padding(.top, 10)

            HStack {
                Text("How much would you like to send?")
                    .font(.callout)
                    .foregroundStyle(.secondary.opacity(0.80))
                    .fontWeight(.medium)
                Spacer()
            }
        }
        .padding(.top)
    }

    @ViewBuilder
    var EnterAmountSection: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                TextField("", text: $sendAmount)
                    .focused($focusField, equals: .amount)
                    .multilineTextAlignment(.center)
                    .font(.system(size: 48, weight: .bold))
                    .keyboardType(metadata.selectedUnit == .btc ? .decimalPad : .numberPad)
                    .offset(x: screenWidth * 0.06)
                    .padding(.horizontal, 30)
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)

                Text(model.unit)
                    .padding(.vertical, 10)
                    .contentShape(
                        .contextMenuPreview, RoundedRectangle(cornerRadius: 8).inset(by: -5)
                    )
                    .contextMenu(
                        menuItems: {
                            Button {
                                model.dispatch(action: .updateUnit(.btc))
                            } label: {
                                Text("btc")
                            }

                            Button {
                                model.dispatch(action: .updateUnit(.sat))
                            } label: {
                                Text("sats")
                            }
                        },
                        preview: {
                            Text(model.unit)
                                .padding(12)
                                .clipShape(RoundedRectangle(cornerRadius: 8))
                        }
                    )
                    .offset(y: -1)
            }

            Text(sendAmountFiat)
                .font(.title3)
                .foregroundColor(.secondary)
        }
        .padding(.vertical, 4)
        .onChange(of: metadata.selectedUnit, initial: false, selectedUnitChanged)
        .onChange(of: sendAmount, sendAmountChanged)
    }

    @ViewBuilder
    var NetworkFeeSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Network Fee")
                .font(.headline)
                .foregroundColor(.secondary)

            HStack {
                Text("2 hours")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Button("Change speed") {
                    // Action
                }
                .font(.caption)
                .foregroundColor(.blue)

                Spacer()

                Text("300 sats")
                    .foregroundStyle(.secondary)
                    .fontWeight(.medium)
            }
        }
        .padding(.top, 12)
    }

    @ViewBuilder
    var EnterAddressSection: some View {
        VStack(spacing: 8) {
            HStack {
                Text("Set address")
                    .font(.headline)
                    .fontWeight(.bold)

                Spacer()
            }
            .id(FocusField.address)
            .padding(.top, 10)

            HStack {
                Text("Where do you want to send to?")
                    .font(.callout)
                    .foregroundStyle(.secondary.opacity(0.80))
                    .fontWeight(.medium)
                Spacer()

                Button(action: {}) {
                    Image(systemName: "qrcode")
                }
                .foregroundStyle(.secondary)
                .foregroundStyle(.secondary)
            }

            HStack {
                TextEditor(text: $address)
                    .focused($focusField, equals: .address)
                    .frame(height: 40)
                    .font(.system(size: 16, design: .none))
                    .foregroundStyle(.primary.opacity(0.9))
            }
        }
        .padding(.top, 14)
    }

    @ViewBuilder
    var AccountSection: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Image(systemName: "bitcoinsign")
                    .font(.title2)
                    .foregroundColor(.orange)
                    .padding(.trailing, 6)

                VStack(alignment: .leading, spacing: 6) {
                    Text("73C5DA0A")
                        .font(.footnote)
                        .foregroundColor(.secondary)

                    Text("Daily Spending Wallet")
                        .font(.headline)
                        .fontWeight(.medium)
                }

                Spacer()
            }
            .padding()
            //                        .background(Color(.systemGray6))
            .cornerRadius(12)
        }
    }

    @ViewBuilder
    var TotalSection: some View {
        HStack {
            Text("Total Spent")
                .font(.title3)
                .fontWeight(.medium)

            Spacer()

            Text(sendAmount)
                .multilineTextAlignment(.center)
                .font(.title3)
                .fontWeight(.medium)
        }
        .padding(.top, 12)
    }

    @ViewBuilder
    var NextButtonBottom: some View {
        Button(action: {
            // Action
        }) {
            Text("Next")
                .font(.title3)
                .fontWeight(.semibold)
                .frame(maxWidth: .infinity)
                .padding()
                .background(Color.midnightBlue)
                .foregroundColor(.white)
                .cornerRadius(10)
        }
        .padding(.top, 8)
        .padding(.bottom)
    }
}

#Preview {
    NavigationStack {
        AsyncPreview {
            SendFlowSetAmountScreen(
                id: WalletId(), model: WalletViewModel(preview: "preview_only"),
                address: "bc1q08uzlzk9lzq2an7gfn3l4ejglcjgwnud9jgqpc"
            )
            .environment(MainViewModel())
        }
    }
}
