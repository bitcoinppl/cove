//
//  SendFlowConfirmScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowConfirmScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth

    let id: WalletId
    @State var manager: WalletManager
    let details: ConfirmDetails
    let prices: PriceResponse? = nil

    // private
    @State private var isShowingAlert = false
    @State private var sendState: SendState = .idle
    @State private var btcOrFiat = FiatOrBtc.btc

    // popover to change btc and sats
    @State private var showingMenu: Bool = false

    func toggleFiatOrBtc() {
        let opposite = btcOrFiat == .btc ? FiatOrBtc.fiat : FiatOrBtc.btc
        btcOrFiat = opposite
    }

    var fiatAmount: String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = details.sendingAmount().asBtc() * Double(prices.usd)

        return manager.fiatAmountToString(amount)
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

            // MARK: CONTENT

            ScrollView {
                VStack(spacing: 24) {
                    // set amount
                    VStack(spacing: 8) {
                        HStack {
                            Text("You're sending")
                                .font(.headline)
                                .fontWeight(.bold)

                            Spacer()
                        }
                        .padding(.top, 6)

                        HStack {
                            Text("The amount they will receive")
                                .font(.footnote)
                                .foregroundStyle(.secondary.opacity(0.80))
                                .fontWeight(.medium)
                            Spacer()
                        }
                    }
                    .padding(.top, 10)

                    // the amount in sats or btc
                    VStack(spacing: 8) {
                        HStack(alignment: .bottom) {
                            Spacer()

                            Text(manager.amountFmt(details.sendingAmount()))
                                .font(.system(size: 48, weight: .bold))
                                .minimumScaleFactor(0.01)
                                .lineLimit(1)
                                .multilineTextAlignment(.trailing)
                                .padding(.trailing, metadata.selectedUnit == .sat ? 10 : 2)

                            Button(action: { showingMenu.toggle() }) {
                                HStack(spacing: 0) {
                                    Text(metadata.selectedUnit == .sat ? "sats" : "btc")

                                    Image(systemName: "chevron.down")
                                        .font(.caption)
                                        .fontWeight(.bold)
                                        .padding(.top, 2)
                                        .padding(.leading, 4)
                                }
                                .frame(alignment: .trailing)
                            }
                            .foregroundStyle(.primary)
                            .padding(.vertical, 10)
                            .padding(.leading, 16)
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
                        .frame(alignment: .trailing)

                        Text(fiatAmount)
                            .font(.title3)
                            .foregroundColor(.secondary)
                    }
                    .padding(.top, 8)

                    SendFlowAccountSection(manager: manager)
                        .padding(.top)

                    Divider()

                    SendFlowDetailsView(manager: manager, details: details)
                }
            }
            .scrollIndicators(.hidden)
            .background(Color.coveBg)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal)

            SwipeToSendView(sendState: $sendState) {
                sendState = .sending
                Task {
                    do {
                        _ = try await manager.rust.signAndBroadcastTransaction(psbt: details.psbt())
                        sendState = .sent
                        isShowingAlert = true
                    } catch {
                        sendState = .error
                    }
                }
            }
            .padding(.horizontal)
            .padding(.bottom, 6)
            .padding(.top, 20)
            .background(Color.coveBg)
            .onAppear {
                // accessing seed words for signing, lock so we can re-auth
                if metadata.walletType == .hot { auth.lock() }
            }
            .alert(
                "Sent!",
                isPresented: $isShowingAlert,
                actions: {
                    Button("OK") {
                        app.resetRoute(to: Route.selectedWallet(id))
                    }
                },
                message: {
                    Text("Transaction was successfully sent!")
                }
            )
        }
    }
}

#Preview {
    NavigationStack {
        AsyncPreview {
            SendFlowConfirmScreen(
                id: WalletId(),
                manager: WalletManager(preview: "preview_only"),
                details: ConfirmDetails.previewNew()
            )
            .environment(AppManager())
            .environment(AuthManager())
        }
    }
}
