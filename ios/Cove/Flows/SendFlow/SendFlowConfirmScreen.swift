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

                    // Balance Section
                    VStack(spacing: 8) {
                        HStack(alignment: .bottom) {
                            Text(manager.amountFmt(details.sendingAmount()))
                                .font(.system(size: 48, weight: .bold))
                                .minimumScaleFactor(0.01)
                                .lineLimit(1)

                            Text(metadata.selectedUnit == .sat ? "sats" : "btc")
                                .padding(.vertical, 10)
                                .padding(.horizontal, 16)
                                .contentShape(
                                    .contextMenuPreview,
                                    RoundedRectangle(cornerRadius: 8)
                                )
                                .contextMenu {
                                    Button {
                                        manager.dispatch(
                                            action: .updateUnit(.sat))
                                    } label: {
                                        Text("sats")
                                    }

                                    Button {
                                        manager.dispatch(
                                            action: .updateUnit(.btc))
                                    } label: {
                                        Text("btc")
                                    }
                                } preview: {
                                    Text(
                                        metadata.selectedUnit == .sat
                                            ? "sats" : "btc"
                                    )
                                    .padding(.vertical, 10)
                                    .padding(.horizontal)
                                }
                                .offset(y: -5)
                                .offset(x: -16)
                        }
                        .offset(x: 32)

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
