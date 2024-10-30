//
//  SendFlowConfirmScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowConfirmScreen: View {
    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView()

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

                            Text("sats")
                                .padding(.bottom, 10)
                        }

                        Text("≈ $326.93 USD")
                            .font(.title3)
                            .foregroundColor(.secondary)
                    }
                    .padding(.top, 8)

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
                        .background(Color(.systemGray6))
                        .cornerRadius(12)
                    }
                    .padding(.vertical, 12)

                    // To Address Section
                    HStack {
                        Text("To Address")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .foregroundColor(.primary)

                        Spacer()

                        Text("bc1q uyye 0qg5 vyd3 e63s 0vus eqod 7h3j 44y1 8h4s 183d x37a")
                            .lineLimit(3, reservesSpace: true)
                            .font(.system(.callout, design: .monospaced))
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
        .navigationTitle("Confirm Transaction")
        .navigationBarTitleDisplayMode(.inline)
        .scrollIndicators(.hidden)
    }
}

#Preview {
    NavigationStack {
        SendFlowConfirmScreen()
    }
}
