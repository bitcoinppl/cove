//
//  SendFlowSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowSetAmountScreen: View {
    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId
    @State var model: WalletViewModel
    @State var address: String = ""

    // private

    // text inputs
    @State private var sendAmount: String = "0"
    @State private var sendAmountFiat: String = "≈ $0.00 USD"

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
                            Text("Set amount")
                                .font(.title3)
                                .fontWeight(.bold)

                            Spacer()
                        }
                        .padding(.top, 10)

                        HStack {
                            Text("How much would you like to send?")
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
                            TextField("", text: $sendAmount)
                                .multilineTextAlignment(.center)
                                .font(.system(size: 48, weight: .bold))
                                .multilineTextAlignment(.center)
                                .keyboardType(.decimalPad)
                                .toolbar {
                                    ToolbarItemGroup(placement: .keyboard) {
                                        Button("Done") {
//                                            isFocused = false
                                        }
                                        .foregroundStyle(.primary)
                                    }
                                }
                                .offset(x: 14)

                            Text("sats")
                                .padding(.bottom, 10)
                        }

                        Text(sendAmountFiat)
                            .font(.title3)
                            .foregroundColor(.secondary)
                    }
                    .padding(.vertical, 8)

                    // Address Section
                    VStack(spacing: 8) {
                        HStack {
                            Text("Set Address")
                                .font(.headline)
                                .fontWeight(.bold)

                            Spacer()
                        }
                        .padding(.top, 10)

                        HStack {
                            Text("Where do you want to send to?")
                                .font(.callout)
                                .foregroundStyle(.secondary.opacity(0.80))
                                .fontWeight(.medium)
                            Spacer()

                            Button(action: {}) {
                                Image(systemName: "qrcode")
                            }
                            .foregroundStyle(.secondary)
                        }

                        HStack {
                            TextEditor(text: $address)
                                .frame(height: 40)
                                .font(.system(size: 16, design: .none))
                                .foregroundStyle(.primary.opacity(0.9))
                        }
                    }
                    .padding(.top, 14)

                    // Account Section
                    VStack(alignment: .leading, spacing: 16) {
                        HStack {
                            Image(systemName: "bitcoinsign")
                                .font(.title2)
                                .foregroundColor(.orange)
                                .padding(.trailing, 6)

                            VStack(alignment: .leading, spacing: 6) {
                                Text("73C5DA0A")
                                    .font(.footnote)
                                    .foregroundColor(.secondary)

                                Text("Daily Spending Wallet")
                                    .font(.headline)
                                    .fontWeight(.medium)
                            }

                            Spacer()
                        }
                        .padding()
                        //                        .background(Color(.systemGray6))
                        .cornerRadius(12)
                    }

                    // Network Fee Section
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Network Fee")
                            .font(.headline)
                            .foregroundColor(.secondary)

                        HStack {
                            Text("2 hours")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Button("Change speed") {
                                // Action
                            }
                            .font(.caption)
                            .foregroundColor(.blue)

                            Spacer()

                            Text("300 sats")
                                .foregroundStyle(.secondary)
                                .fontWeight(.medium)
                        }
                    }
                    .padding(.top, 12)

                    // Total Section
                    HStack {
                        Text("Total Spent")
                            .font(.title3)
                            .fontWeight(.medium)

                        Spacer()

                        Text("573,599")
                            .font(.title3)
                            .fontWeight(.medium)
                    }
                    .padding(.top, 12)

                    Spacer()

                    // Next Button
                    Button(action: {
                        // Action
                    }) {
                        Text("Next")
                            .font(.title3)
                            .fontWeight(.semibold)
                            .frame(maxWidth: .infinity)
                            .padding()
                            .background(Color.midnightBlue)
                            .foregroundColor(.white)
                            .cornerRadius(10)
                    }
                    .padding(.top, 8)
                    .padding(.bottom)
                }
            }
            // </ScrollView>
            .padding(.horizontal)
            .frame(width: screenWidth)
            .background(colorScheme == .light ? .white : .black)
            .scrollIndicators(.hidden)
        }
        // </VStack>
        .padding(.top, 0)
        .navigationTitle("Send")
        .navigationBarTitleDisplayMode(.inline)
    }
}

#Preview {
    NavigationStack {
        AsyncPreview {
            SendFlowSetAmountScreen(
                id: WalletId(), model: WalletViewModel(preview: "preview_only"),
                address: "bc1q08uzlzk9lzq2an7gfn3l4ejglcjgwnud9jgqpc"
            )
            .environment(MainViewModel())
        }
    }
}
