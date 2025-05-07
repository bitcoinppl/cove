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

    let sendFlowManager: SendFlowManager

    @State var enteringBtcAmount: String
    @State var enteringFiatAmount: String

    @FocusState private var focusField: SendFlowPresenter.FocusField?
    @State private var showingMenu: Bool = false

    init(sendFlowManager: SendFlowManager) {
        self.sendFlowManager = sendFlowManager
        self.enteringBtcAmount = sendFlowManager.enteringBtcAmount
        self.enteringFiatAmount = sendFlowManager.enteringFiatAmount
    }

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

    var sendAmountFiat: String {
        let _ = sendFlowManager.address
        let _ = sendFlowManager.feeRateOptions
        let _ = sendFlowManager.selectedFeeRate
        let _ = sendFlowManager.amount
        return sendFlowManager.rust.sendAmountFiat()
    }

    var sendAmountBtc: String {
            let _ = sendFlowManager.address
            let _ = sendFlowManager.feeRateOptions
            let _ = sendFlowManager.selectedFeeRate
            let _ = sendFlowManager.amount
        return sendFlowManager.rust.sendAmountBtc()
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
                    .onChange(of: enteringBtcAmount, initial: false) { oldValue, newValue in
                        Log.debug("onChange \(oldValue) -> \(newValue)")
                        if oldValue == newValue { return }
                        if let newEnteringAmount = sendFlowManager.rust.sanitizeBtcEnteringAmount(
                            oldValue: oldValue, newValue: newValue),
                            newValue != newEnteringAmount
                        {
                            Log.debug(
                                "btcEntering \(oldValue) -> \(newValue) -> \(newEnteringAmount)")
                            enteringBtcAmount = newEnteringAmount
                            return
                        }
                        
                        if sendFlowManager.enteringBtcAmount == newValue { return }
                        sendFlowManager.dispatch(action: .changeEnteringBtcAmount(newValue))
                    }
                    .onChange(of: enteringFiatAmount, initial: false) { oldValue, newValue in
                        Log.debug("onChange \(oldValue) -> \(newValue)")
                        if oldValue == newValue { return }
                        if let newEnteringAmount =
                            sendFlowManager.rust.sanitizeFiatEnteringAmount(
                                oldValue: oldValue, newValue: newValue),
                            newValue != newEnteringAmount
                        {
                            Log.debug(
                                "fiatEntering \(oldValue) -> \(newValue) -> \(newEnteringAmount)")
                            enteringFiatAmount = newEnteringAmount
                            return
                        }

                        if sendFlowManager.enteringFiatAmount == newValue { return }
                        sendFlowManager.dispatch(action: .changeEnteringFiatAmount(newValue))
                    }
                    .onChange(of: sendFlowManager.enteringBtcAmount, initial: true) {
                        oldValue, newValue in
                        Log.debug("enteringBtcAmount \(oldValue) -> \(newValue)")
                        enteringBtcAmount = newValue
                    }
                    .onChange(of: sendFlowManager.enteringFiatAmount, initial: true) {
                        oldValue, newValue in
                        Log.debug("enteringFiatAmount \(oldValue) -> \(newValue)")
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
                .onChange(of: focusField, initial: true) { _, new in focusField = new }
                .popover(isPresented: $showingMenu) {
                    VStack(alignment: .center, spacing: 0) {
                        Button(action: {
                            manager.dispatch(action: .updateUnit(.sat))
                            showingMenu = false
                        }) {
                            Text("sats")
                                .frame(maxWidth: .infinity)
                                .padding(12)
                                .background(Color.clear)
                        }
                        .buttonStyle(.plain)
                        .contentShape(Rectangle())

                        Divider()

                        Button(action: {
                            manager.dispatch(action: .updateUnit(.btc))
                            showingMenu = false
                        }) {
                            Text("btc")
                                .frame(maxWidth: .infinity)
                                .padding(12)
                                .background(Color.clear)
                        }
                        .buttonStyle(.plain)
                        .contentShape(Rectangle())
                    }
                    .padding(.vertical, 8)
                    .padding(.horizontal, 12)
                    .frame(minWidth: 120, maxWidth: 200)
                    .presentationCompactAdaptation(.popover)
                    .foregroundStyle(.primary.opacity(0.8))
                    .contentShape(Rectangle())
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
