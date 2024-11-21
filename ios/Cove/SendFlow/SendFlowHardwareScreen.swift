//
//  SendFlowHardwareScreen.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import Foundation
import SwiftUI

private enum SheetState: Equatable {
    case details
}

struct SendFlowHardwareScreen: View {
    @Environment(MainViewModel.self) private var app

    let id: WalletId
    @State var model: WalletViewModel
    let details: ConfirmDetails
    let prices: PriceResponse? = nil

    // private
    @State private var sheetState: TaggedItem<SheetState>? = .none

    var metadata: WalletMetadata {
        model.walletMetadata
    }

    var fiatAmount: String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = details.sendingAmount().asBtc() * Double(prices.usd)
        return model.fiatAmountToString(amount)
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(model: model, amount: model.balance.confirmed)

            // MARK: CONTENT

            ScrollView {
                VStack(spacing: 24) {
                    // amount
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

                    AccountSection.padding(.vertical)

                    Divider()

                    // MARK: To Address Section

                    HStack {
                        Text("Address")
                            .font(.footnote)
                            .fontWeight(.medium)
                            .foregroundStyle(.secondary)
                            .foregroundColor(.primary)

                        Spacer()
                        Spacer()
                        Spacer()
                        Spacer()

                        Text(
                            details.sendingTo().spacedOut()
                        )
                        .lineLimit(4, reservesSpace: false)
                        .font(.system(.footnote, design: .none))
                        .fontWeight(.semibold)
                        .padding(.leading, 60)
                    }
                    .padding(.vertical, 8)

                    Divider()

                    // sign Transaction Section
                    SignTransactionSection

                    Spacer()

                    // more details button
                    Button(action: { sheetState = .init(.details) }) {
                        Text("More details")
                    }
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                }
            }
            .scrollIndicators(.hidden)
            .background(Color.coveBg)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal)
            .sheet(item: $sheetState, content: SheetContent)
        }
    }

    @ViewBuilder
    var AccountSection: some View {
        VStack {
            HStack {
                BitcoinShieldIcon(width: 24, color: .orange)

                VStack(alignment: .leading, spacing: 6) {
                    Text(
                        metadata.masterFingerprint?.asUppercase()
                            ?? "No Fingerprint"
                    )
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundColor(.secondary)

                    Text(metadata.name)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
                .padding(.leading, 8)

                Spacer()
            }
        }
    }

    @ViewBuilder
    var SignTransactionSection: some View {
        VStack(spacing: 17) {
            HStack {
                Text("Sign Transaction")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundColor(.secondary)

                Spacer()
            }

            HStack {
                Button(action: {
                    // Export transaction action
                }) {
                    Label("Export Transaction", systemImage: "square.and.arrow.up")
                        .padding(.horizontal, 18)
                        .padding(.vertical)
                        .foregroundColor(.midnightBlue)
                        .background(.buttonPrimary)
                        .cornerRadius(8)
                        .font(.caption)
                }

                Spacer()

                Button(action: {
                    // Import signature action
                }) {
                    Label("Import Signature", systemImage: "square.and.arrow.down")
                        .padding(.horizontal, 18)
                        .padding(.vertical)
                        .foregroundColor(.midnightBlue)
                        .background(.buttonPrimary)
                        .cornerRadius(8)
                        .font(.caption)
                }
            }
        }
    }

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .details:
            SendFlowDetailsSheetView(model: model, details: details)
                .presentationDetents([.height(425), .height(600), .large])
                .padding()
        }
    }
}

#Preview {
    NavigationStack {
        AsyncPreview {
            SendFlowHardwareScreen(
                id: WalletId(),
                model: WalletViewModel(preview: "preview_only"),
                details: ConfirmDetails.previewNew()
            )
            .environment(MainViewModel())
        }
    }
}
