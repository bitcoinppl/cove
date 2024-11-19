//
//  SendFlowConfirmScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowConfirmScreen: View {
    @Environment(MainViewModel.self) private var app

    let id: WalletId
    @State var model: WalletViewModel
    let details: ConfirmDetails

    var fiatAmount: String {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = details.sendingAmount().asBtc() * Double(prices.usd)

        return "≈ \(amount) USD"
    }

    var metadata: WalletMetadata {
        model.walletMetadata
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(model: model, amount: model.balance.confirmed)

            // MARK: CONTENT

            ScrollView {
                VStack(spacing: 24) {
                    // set amount
                    VStack(spacing: 8) {
                        HStack {
                            Text("You're sending")
                                .font(.title3)
                                .fontWeight(.bold)

                            Spacer()
                        }
                        .padding(.top, 10)

                        HStack {
                            Text("The amount they will receive")
                                .font(.callout)
                                .foregroundStyle(.secondary.opacity(0.80))
                                .fontWeight(.medium)
                            Spacer()
                        }
                    }
                    .padding(.top)

                    // Balance Section
                    VStack(spacing: 8) {
                        HStack(alignment: .bottom) {
                            Text(model.amountFmt(details.sendingAmount()))
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
                                        model.dispatch(
                                            action: .updateUnit(.sat))
                                    } label: {
                                        Text("sats")
                                    }

                                    Button {
                                        model.dispatch(
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

                    AccountSection
                        .padding(.top)

                    Divider()

                    // To Address Section
                    HStack {
                        Text("To Address")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .foregroundColor(.primary)

                        Spacer()
                        Spacer()

                        Text(
                            details.sendingTo().spacedOut()
                        )
                        .lineLimit(2, reservesSpace: true)
                        .font(.system(.callout, design: .none))
                        .fontWeight(.medium)
                        .padding(.leading, 60)
                    }
                    .padding(.top, 6)

                    // Network Fee Section
                    HStack {
                        Text("Network Fee")
                            .font(.callout)
                            .foregroundStyle(.secondary)

                        Spacer()

                        HStack {
                            Text(model.amountFmt(details.sendingAmount()))
                            Text(metadata.selectedUnit == .sat ? "sats" : "btc")
                        }
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    }

                    // Total Amount Section
                    HStack {
                        Text("You'll pay")
                            .fontWeight(.medium)
                        Spacer()
                        HStack {
                            Text(model.amountFmt(details.spendingAmount()))
                                .fontWeight(.semibold)
                            Text(metadata.selectedUnit == .sat ? "sats" : "btc")
                        }
                    }
                }
            }
            .scrollIndicators(.hidden)
            .background(Color.coveBg)
            .padding(.horizontal)
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            SwipeToSendView()
                .padding(.bottom)
                .padding(.horizontal)
                .background(Color.coveBg)
        }
    }

    @ViewBuilder
    var AccountSection: some View {
        VStack(spacing: 16) {
            HStack {
                Spacer()

                Image(systemName: "bitcoinsign")
                    .font(.title2)
                    .foregroundColor(.orange)
                    .padding(.trailing, 6)

                VStack(alignment: .leading, spacing: 6) {
                    Text(
                        metadata.masterFingerprint?.asUppercase()
                            ?? "No Fingerprint"
                    )
                    .font(.footnote)
                    .foregroundColor(.secondary)

                    Text(metadata.name)
                        .font(.headline)
                        .fontWeight(.medium)
                }
                .padding(.leading, 24)

                Spacer()
                Spacer()
                Spacer()
                Spacer()
                Spacer()
                Spacer()
            }
            .cornerRadius(12)
        }
    }
}

#Preview {
    NavigationStack {
        AsyncPreview {
            SendFlowConfirmScreen(
                id: WalletId(),
                model: WalletViewModel(preview: "preview_only"),
                details: ConfirmDetails.previewNew()
            )
            .environment(MainViewModel())
        }
    }
}
