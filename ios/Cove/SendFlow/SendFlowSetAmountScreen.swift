//
//  SendFlowSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowSetAmountScreen: View {
    let id: WalletId
    let model: WalletViewModel

    var metadata: WalletMetadata {
        model.walletMetadata
    }

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
                    .padding(.vertical, 8)
                    
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
            .padding()
            .frame(width: screenWidth)

            Spacer()
        }
        .navigationTitle("Send")
        .navigationBarTitleDisplayMode(.inline)
        .scrollIndicators(.hidden)
    }
}

#Preview {
    NavigationStack {
        AsyncPreview {
            SendFlowSetAmountScreen(id: WalletId(), model: WalletViewModel(preview: "preview_only"))
                .environment(MainViewModel())
        }
    }
}
