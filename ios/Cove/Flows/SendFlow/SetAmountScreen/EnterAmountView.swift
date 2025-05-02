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
    @Environment(SendFlowManager.self) private var sendFlowManager

    @FocusState private var focusField: SendFlowPresenter.FocusField?
    @State private var showingMenu: Bool = false

    @State private var enteringBtcAmount: String = ""
    @State private var enteringFiatAmount: String = ""

    private var enteringAmount: Binding<String> {
        switch metadata.fiatOrBtc {
        case .btc: $enteringBtcAmount
        case .fiat: $enteringFiatAmount
        }
    }

    var metadata: WalletMetadata { manager.walletMetadata }

    var offset: CGFloat {
        if metadata.fiatOrBtc == .fiat { return 0 }
        return metadata.selectedUnit == .btc ? screenWidth * 0.10 : screenWidth * 0.11
    }

    var textField: String {
        if metadata.fiatOrBtc == .btc { return sendFlowManager.enteringBtcAmount }
        return sendFlowManager.enteringFiatAmount
    }

    var sendAmountFiat: String {
        sendFlowManager.rust.sendAmountFiat(amountSats: sendFlowManager.amount?.asSats())
    }

    var sendAmountBtc: String {
        sendFlowManager.rust.sendAmountBtc(amountSats: sendFlowManager.amount?.asSats())
    }

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                TextField("", text: enteringAmount)
                    .font(.system(size: 48, weight: .bold))
                    .multilineTextAlignment(.center)
                    .keyboardType(.decimalPad)
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)
                    .scrollDisabled(true)
                    .offset(x: offset)
                    .padding(.horizontal, 30)
                    .focused($focusField, equals: .amount)
                    .onChange(of: enteringBtcAmount, initial: true) { oldValue, newValue in
                        if let newEnteringAmount =
                            sendFlowManager.rust.sanitizeBtcEnteringAmount(old: oldValue, new: newValue)
                        {
                            return enteringBtcAmount = newEnteringAmount
                        }

                        sendFlowManager.dispatch(action: .changeEnteringBtcAmount(newValue))
                    }
                    .onChange(of: enteringFiatAmount, initial: true) { oldValue, newValue in
                        if let newEnteringAmount =
                            sendFlowManager.rust.sanitizeFiatEnteringAmount(old: oldValue, new: newValue)
                        {
                            return enteringFiatAmount = newEnteringAmount
                        }

                        sendFlowManager.dispatch(action: .changeEnteringFiatAmount(newValue))
                    }
                    .onChange(of: sendFlowManager.enteringBtcAmount, initial: true) { _, newValue in
                        enteringBtcAmount = newValue
                    }
                    .onChange(of: sendFlowManager.enteringFiatAmount, initial: true) { _, newValue in
                        enteringFiatAmount = newValue
                    }

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
                .onChange(of: focusField, initial: true) { _, newFocusField in
                    guard let newFocusField else { return }
                    presenter.focusField = newFocusField
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
                Text(metadata.fiatOrBtc == .btc ? sendAmountFiat : sendAmountBtc)
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
