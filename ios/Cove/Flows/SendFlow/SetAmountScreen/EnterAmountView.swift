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

    @FocusState private var focusField: SendFlowPresenter.FocusField?
    @State private var showingMenu: Bool = false

    init(sendFlowManager: SendFlowManager) {
        self.sendFlowManager = sendFlowManager
    }

    private var enteringFiatBinding: Binding<String> {
        Binding(
            get: { sendFlowManager.enteringFiatAmount },
            set: { new in
                let clean = sendFlowManager.rust.sanitizeFiatEnteringAmount(
                    oldValue: sendFlowManager.enteringFiatAmount,
                    newValue: new
                ) ?? new
                if sendFlowManager.enteringFiatAmount != clean {
                    sendFlowManager.enteringFiatAmount = clean
                    sendFlowManager.dispatch(action: .notifyEnteringFiatAmountChanged(clean))
                }
            }
        )
    }

    private var enteringBtcBinding: Binding<String> {
        Binding(
            get: { sendFlowManager.enteringBtcAmount },
            set: { new in
                let clean = sendFlowManager.rust.sanitizeBtcEnteringAmount(
                    oldValue: sendFlowManager.enteringBtcAmount,
                    newValue: new
                ) ?? new

                if sendFlowManager.enteringBtcAmount != clean {
                    sendFlowManager.enteringBtcAmount = clean
                    sendFlowManager.dispatch(action: .notifyEnteringBtcAmountChanged(clean))
                }
            }
        )
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

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                switch metadata.fiatOrBtc {
                case .btc:
                    TextField("", text: enteringBtcBinding)
                        .font(.system(size: 48, weight: .bold))
                        .multilineTextAlignment(.center)
                        .keyboardType(.decimalPad)
                        .minimumScaleFactor(0.01)
                        .lineLimit(1)
                        .scrollDisabled(true)
                        .offset(x: offset)
                        .padding(.horizontal, 30)
                        .focused($focusField, equals: .amount)

                case .fiat:
                    TextField("", text: enteringFiatBinding)
                        .font(.system(size: 48, weight: .bold))
                        .multilineTextAlignment(.center)
                        .keyboardType(.decimalPad)
                        .minimumScaleFactor(0.01)
                        .lineLimit(1)
                        .scrollDisabled(true)
                        .offset(x: offset)
                        .padding(.horizontal, 30)
                        .focused($focusField, equals: .amount)
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
                .onChange(of: presenter.focusField, initial: true) {
                    _, new in focusField = new
                }
                .onChange(of: focusField, initial: true) { _, new in
                    if new == .none {
                        focusField = presenter.focusField
                    } else {
                        presenter.focusField = new
                    }
                }
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
