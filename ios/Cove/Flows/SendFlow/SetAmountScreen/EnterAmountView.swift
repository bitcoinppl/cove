//
//  SendFlowEnterAmountView.swift
//  Cove
//
//  Created by Praveen Perera on 11/19/24.
//
import SwiftUI

struct EnterAmountView: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(SendFlowPresenter.self) private var presenter
    @Environment(WalletManager.self) private var manager

    let sendFlowManager: SendFlowManager

    @State var enteringBtcAmount: String
    @State var enteringFiatAmount: String

    @FocusState private var focusField: SendFlowPresenter.FocusField?
    @State private var showingMenu: Bool = false
    @State private var previouslyExceeded: Bool = false

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
        sendFlowManager.sendAmountFiat
    }

    var sendAmountBtc: String {
        sendFlowManager.sendAmountBtc
    }

    var exceedsBalance: Bool {
        sendFlowManager.rust.amountExceedsBalance()
    }

    var amountTextColor: Color {
        exceedsBalance ? .orange : .primary
    }

    @ViewBuilder
    private var btcTextField: some View {
        TextField("", text: $enteringBtcAmount)
            .font(.system(size: 48, weight: .bold))
            .foregroundColor(amountTextColor)
            .multilineTextAlignment(.center)
            .keyboardType(.decimalPad)
            .minimumScaleFactor(0.01)
            .lineLimit(1)
            .scrollDisabled(true)
            .offset(x: offset)
            .padding(.horizontal, 30)
            .focused($focusField, equals: .amount)
            .frame(height: UIFont.boldSystemFont(ofSize: 48).lineHeight)
    }

    @ViewBuilder
    private var fiatTextField: some View {
        TextField("", text: $enteringFiatAmount)
            .font(.system(size: 48, weight: .bold))
            .foregroundColor(amountTextColor)
            .multilineTextAlignment(.center)
            .keyboardType(.decimalPad)
            .minimumScaleFactor(0.01)
            .lineLimit(1)
            .scrollDisabled(true)
            .offset(x: offset)
            .padding(.horizontal, 30)
            .focused($focusField, equals: .amount)
            .frame(height: UIFont.boldSystemFont(ofSize: 48).lineHeight)
    }

    @ViewBuilder
    private var unitSelector: some View {
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
        .popover(isPresented: $showingMenu) {
            unitPopoverContent
        }
    }

    @ViewBuilder
    private var unitPopoverContent: some View {
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

    @ViewBuilder
    private var secondaryAmountRow: some View {
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

    private func handleBtcAmountChange(oldValue: String, newValue: String) {
        Log.debug("onChangeBTC \(oldValue) -> \(newValue) (\(sendFlowManager.enteringBtcAmount))")

        let newEnteringAmount = sendFlowManager.rust.sanitizeBtcEnteringAmount(oldValue: oldValue, newValue: newValue)
        if let newEnteringAmount, newValue != newEnteringAmount {
            enteringBtcAmount = newEnteringAmount
            return
        }

        if sendFlowManager.enteringBtcAmount != newValue {
            sendFlowManager.enteringBtcAmount = newValue
            sendFlowManager.dispatch(action: .notifyEnteringBtcAmountChanged(newValue))
        }
    }

    private func handleFiatAmountChange(oldValue: String, newValue: String) {
        Log.debug("onChangeFiat \(oldValue) -> \(newValue) (\(sendFlowManager.enteringFiatAmount))")

        let newEnteringAmount = sendFlowManager.rust.sanitizeFiatEnteringAmount(oldValue: oldValue, newValue: newValue)
        if let newEnteringAmount, newValue != newEnteringAmount {
            enteringFiatAmount = newEnteringAmount
            return
        }

        if sendFlowManager.enteringFiatAmount != newValue {
            sendFlowManager.enteringFiatAmount = newValue
            sendFlowManager.dispatch(action: .notifyEnteringFiatAmountChanged(newValue))
        }
    }

    private func handleExceedsBalanceChange(newValue: Bool) {
        if newValue, !previouslyExceeded {
            Task {
                await FloaterPopup(
                    text: "Exceeds available balance",
                    backgroundColor: .orange,
                    textColor: .white,
                    iconColor: .white,
                    icon: "exclamationmark.triangle"
                )
                .dismissAfter(2)
                .present()
            }
        }
        previouslyExceeded = newValue
    }

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                switch metadata.fiatOrBtc {
                case .btc: btcTextField
                case .fiat: fiatTextField
                }
                unitSelector
            }
            .onChange(of: enteringBtcAmount, initial: false) { oldValue, newValue in
                handleBtcAmountChange(oldValue: oldValue, newValue: newValue)
            }
            .onChange(of: enteringFiatAmount, initial: false) { oldValue, newValue in
                handleFiatAmountChange(oldValue: oldValue, newValue: newValue)
            }
            .onChange(of: sendFlowManager.enteringBtcAmount, initial: true) { oldValue, newValue in
                Log.debug("enteringBtcAmount \(oldValue) -> \(newValue) (\(enteringBtcAmount))")
                guard enteringBtcAmount != newValue else { return }
                enteringBtcAmount = newValue
            }
            .onChange(of: sendFlowManager.enteringFiatAmount, initial: true) { oldValue, newValue in
                Log.debug("enteringFiatAmount \(oldValue) -> \(newValue) (\(enteringFiatAmount))")
                guard enteringFiatAmount != newValue else { return }
                enteringFiatAmount = newValue
            }
            .onChange(of: presenter.focusField, initial: true) { _, new in
                focusField = new
            }
            .onChange(of: focusField, initial: true) { _, new in
                if auth.lockState == .locked {
                    focusField = .none
                    presenter.focusField = .none
                    return
                }

                if new == .none {
                    focusField = presenter.focusField
                } else {
                    presenter.focusField = new
                }
            }
            .onChange(of: auth.lockState, initial: true) { _, new in
                if new == .unlocked {
                    if !sendFlowManager.rust.validateAmount() {
                        sendFlowManager.dispatch(.changeSetAmountFocusField(.amount))
                        return
                    }

                    if !sendFlowManager.rust.validateAddress() {
                        sendFlowManager.dispatch(.changeSetAmountFocusField(.address))
                        return
                    }
                }
            }
            .onChange(of: exceedsBalance) { _, newValue in
                handleExceedsBalanceChange(newValue: newValue)
            }

            secondaryAmountRow
        }
    }
}
