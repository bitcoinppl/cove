//
//  SendFlowEnterAmountView.swift
//  Cove
//
//  Created by Praveen Perera on 11/19/24.
//
import SwiftUI

struct EnterAmountView: View {
    @Environment(AppManager.self) private var app
    @Environment(SendFlowPresenter.self) private var presenter
    @Environment(WalletManager.self) private var manager

    // args
    @Binding var sendAmount: String
    @Binding var sendAmountFiat: String

    // private

    // private state for entering sendAmountFiat, don't show sendAmountFiat update
    @State private var fiatText: String = ""
    @FocusState private var focusField: SendFlowPresenter.FocusField?
    @State private var showingMenu: Bool = false

    var metadata: WalletMetadata { manager.walletMetadata }

    var offset: CGFloat {
        if metadata.fiatOrBtc == .fiat { return 0 }
        return metadata.selectedUnit == .btc ? screenWidth * 0.10 : screenWidth * 0.11
    }

    var textField: Binding<String> {
        if metadata.fiatOrBtc == .btc { return $sendAmount }
        return $fiatText
    }

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                TextField("", text: textField)
                    .focused($focusField, equals: .amount)
                    .multilineTextAlignment(.center)
                    .font(.system(size: 48, weight: .bold))
                    .keyboardType(.decimalPad)
                    .offset(x: offset)
                    .padding(.horizontal, 30)
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)
                    .scrollDisabled(true)

                HStack(spacing: 0) {
                    if metadata.fiatOrBtc == .btc {
                        Button(action: { showingMenu.toggle() }) {
                            Text(manager.unit)
                                .padding(.vertical, 10)

                            Image(systemName: "chevron.down")
                                .font(.caption)
                                .fontWeight(.bold)
                                .padding(.top, 2)
                                .padding(.leading, 4)
                        }
                        .foregroundStyle(.primary)
                    }
                }
                .onChange(of: presenter.focusField, initial: true) { _, new in focusField = new }
                .onChange(of: focusField, initial: true) { oldFocusField, newFocusField in
                    guard let newFocusField else { return }
                    presenter.focusField = newFocusField

                    // focusField changed when entering btc/sats
                    if metadata.fiatOrBtc == .btc {
                        let sendAmount = sendAmount.replacingOccurrences(of: ",", with: "")
                        if newFocusField == .amount { self.sendAmount = sendAmount }

                        if newFocusField != .amount, metadata.selectedUnit == .sat,
                           let amountInt = Int(sendAmount)
                        {
                            self.sendAmount = ThousandsFormatter(amountInt).fmt()
                        }
                    }

                    // focusField changed when entering fiat
                    if metadata.fiatOrBtc == .fiat {
                        if newFocusField == .amount {
                            do {
                                if fiatText == "" { return }
                                let fiatValue = try Converter().getFiatValue(fiatAmount: fiatText)
                                let fiatAmount = manager.rust.displayFiatAmount(
                                    amount: fiatValue, withSuffix: false
                                )
                                fiatText = fiatAmount
                            } catch {
                                Log.error(
                                    "'EnterAmountView' failed to convert fiat amount (\(fiatText)) to btc: \(error)"
                                )
                            }
                        }

                        if oldFocusField == .amount, newFocusField != .amount {
                            do {
                                let fiatValue = try Converter().getFiatValue(fiatAmount: fiatText)
                                let fiatAmount = manager.rust.displayFiatAmount(
                                    amount: fiatValue, withSuffix: false
                                )

                                sendAmountFiat = fiatAmount
                                fiatText = fiatAmount
                            } catch {
                                Log.error(
                                    "'EnterAmountView' failed to convert fiat amount (\(fiatText)) to btc: \(error)"
                                )
                            }
                        }
                    }
                }
                .onChange(of: fiatText, initial: true) { oldValue, newValue in
                    guard metadata.fiatOrBtc == .fiat else { return }
                    guard let prices = app.prices else { return }

                    // note: using contains instead of direct comparison because need to support multiple currencies
                    // on first number if erasing, erase the entire thing,
                    if oldValue.contains("0.00"), newValue.contains("0.0"), !newValue.contains("0.00"), sendAmount == "0" {
                        return fiatText = newValue.replacingOccurrences(of: "0.0", with: "")
                    }

                    // on first enter, use just the entered number
                    if oldValue.contains("0.00"), newValue.contains("0.00"), sendAmount == "0" {
                        return fiatText = newValue.replacingOccurrences(of: "0.00", with: "")
                    }

                    do {
                        let amount = manager.rust.convertFromFiatString(fiatAmount: newValue, prices: prices)
                        presenter.amount = amount
                        sendAmount = manager.walletMetadata.selectedUnit == .btc ? amount.btcString() : ThousandsFormatter(amount.asSats()).fmt()

                        let fiatValue = try Converter().getFiatValue(fiatAmount: newValue)
                        sendAmountFiat = manager.rust.displayFiatAmount(amount: fiatValue)
                    } catch {
                        Log.error(
                            "'EnterAmountView' failed to convert fiat amount to btc: \(error)")
                    }
                }
                .onChange(of: metadata.fiatOrBtc, initial: true) { old, new in
                    if old == .btc, new == .fiat {
                        fiatText = Converter().removeFiatSuffix(fiatAmount: sendAmountFiat)
                    }
                }
                .onAppear {
                    fiatText = Converter().removeFiatSuffix(fiatAmount: sendAmountFiat)
                }
                .popover(isPresented: $showingMenu) {
                    VStack(alignment: .center, spacing: 0) {
                        Button("sats") {
                            manager.dispatch(action: .updateUnit(.sat))
                            showingMenu = false
                        }
                        .padding(12)
                        .buttonStyle(.plain)

                        Divider()

                        Button("btc") {
                            manager.dispatch(action: .updateUnit(.btc))
                            showingMenu = false
                        }
                        .padding(12)
                        .buttonStyle(.plain)
                    }
                    .padding(.vertical, 8)
                    .padding(.horizontal, 12)
                    .frame(minWidth: 120, maxWidth: 200)
                    .presentationCompactAdaptation(.popover)
                    .foregroundStyle(.primary.opacity(0.8))
                }
            }

            HStack(spacing: 4) {
                Text(metadata.fiatOrBtc == .btc ? sendAmountFiat : sendAmount)
                    .font(.subheadline)
                    .foregroundColor(.secondary)

                if metadata.fiatOrBtc == .fiat {
                    Text(manager.unit)
                        .font(.subheadline)
                        .foregroundColor(.secondary)
                }
            }
            .onTapGesture {
                if metadata.fiatOrBtc == .btc, app.prices == nil { return }
                manager.dispatch(action: .toggleFiatOrBtc)
            }
        }
    }
}
