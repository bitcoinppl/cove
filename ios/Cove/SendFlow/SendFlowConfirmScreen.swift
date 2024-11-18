//
//  SendFlowConfirmScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowConfirmScreen: View {
    let id: WalletId
    @State var model: WalletViewModel
    let details: ConfirmDetails

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

                    // Balance Section
                    VStack(spacing: 8) {
                        HStack(alignment: .bottom) {
                            Text("573,299")
                                .font(.system(size: 48, weight: .bold))

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
                                    Text(metadata.selectedUnit == .sat ? "sats" : "btc")
                                        .padding(.vertical, 10)
                                        .padding(.horizontal)
                                }
                                .offset(y: -5)
                                .offset(x: -16)
                        }
                        .offset(x: 32)

                        Text("≈ $326.93 USD")
                            .font(.title3)
                            .foregroundColor(.secondary)
                    }
                    .padding(.top, 8)

                    AccountSection

                    // To Address Section
                    HStack {
                        Text("To Address")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .foregroundColor(.primary)

                        Spacer()

                        Text(
                            "bc1q uyye 0qg5 vyd3 e63s 0vus eqod 7h3j 44y1 8h4s 183d x37a"
                        )
                        .lineLimit(3, reservesSpace: true)
                        .font(.system(.callout, design: .none))
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
                            Text("300")
                            Text("sats")
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
                            Text("573,000")
                                .fontWeight(.semibold)
                            Text("sats")
                        }
                    }

                    SwipeToSendView()
                        .padding(.top, 28)
                }
            }
            .padding()
            .frame(width: screenWidth)

            Spacer()
        }
    }

    @ViewBuilder
    var AccountSection: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
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

                Spacer()
            }
            .padding()
            .background(Color(.systemGray6))
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
