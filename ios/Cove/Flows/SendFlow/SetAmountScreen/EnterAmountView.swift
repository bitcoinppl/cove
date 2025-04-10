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
                    .contentTransition(.numericText())
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
                        if metadata.selectedUnit == .sat, let amountInt = Int(sendAmount) {
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
                                    "'EnterAmountView::onChangeFocusField' failed to convert fiat amount (\(fiatText)) to btc: \(error)"
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
                                    "'EnterAmountView::onChangeFocusField' failed to convert fiat amount (\(fiatText)) to btc: \(error)"
                                )
                            }
                        }
                    }
                }
                .onChange(of: fiatText, initial: true) { oldValue, newValue in
                    Log.debug("EnterAmountView::onChange::fiatText \(oldValue) --> \(newValue)")
                    guard metadata.fiatOrBtc == .fiat else { return }
                    guard let prices = app.prices else { return }
                    let selectedCurrency = Database().globalConfig().selectedFiatCurrency()

                    do {
                        let result = try SendFlowFiatOnChangeHandler(prices: prices, selectedCurrency: selectedCurrency).onChange(oldValue: oldValue, newValue: newValue)
                        if let amount = result.btcAmount {
                            withAnimation {
                                presenter.amount = amount
                                sendAmount =
                                    manager.walletMetadata.selectedUnit == .btc
                                        ? amount.btcString() : ThousandsFormatter(amount.asSats()).fmt()
                            }
                        }

                        if let fiatValue = result.fiatValue {
                            withAnimation {
                                sendAmountFiat = manager.rust.displayFiatAmount(amount: fiatValue)
                            }
                        }

                        if let fiatText = result.fiatText {
                            withAnimation {
                                self.fiatText = fiatText
                            }
                        }
                    } catch let err as SendFlowFiatOnChangeError {
                        Log.error("'EnterAmountView::onChange' error: \(err.describe)")
                    } catch {
                        Log.error("'EnterAmountView::onChange' unknonw error: \(error.localizedDescription)")
                    }
                }
                .onChange(of: metadata.fiatOrBtc, initial: true) { old, new in
                    if old == .btc, new == .fiat {
                        fiatText = sendAmountFiat
                    }

                    if old == .fiat, new == .btc, fiatText == "" {
                        sendAmountFiat = manager.rust.displayFiatAmount(amount: 0)
                    }
                }
                .onChange(of: sendAmountFiat, initial: false) { _, new in
                    guard metadata.fiatOrBtc == .fiat else { return }
                    let selectedCurrency = Database().globalConfig().selectedFiatCurrency()

                    // allow clearing with the clear button
                    if new == selectedCurrency.symbol() {
                        fiatText = selectedCurrency.symbol()
                    }
                }
                .onAppear { fiatText = sendAmountFiat }
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
                    .contentTransition(.numericText())
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
