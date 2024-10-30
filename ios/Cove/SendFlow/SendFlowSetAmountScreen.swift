//
//  SendFlowSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowSetAmountScreen: View {
    func setToolbarAppearence() {
        let appearance = UINavigationBarAppearance()
        appearance.configureWithTransparentBackground()
        appearance.backgroundColor = UIColor.clear
        appearance.titleTextAttributes = [.foregroundColor: UIColor.white]
        appearance.largeTitleTextAttributes = [.foregroundColor: UIColor.white]

        UINavigationBar.appearance().standardAppearance = appearance
        UINavigationBar.appearance().compactAppearance = appearance
        UINavigationBar.appearance().scrollEdgeAppearance = appearance
        UINavigationBar.appearance().tintColor = .white
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            ZStack {
                VStack {
                    HStack {
                        Text("Balance")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                        Spacer()
                    }

                    HStack {
                        Text("5,215,310")
                            .font(.title2)
                            .fontWeight(.bold)
                        Text("sats")
                            .font(.subheadline)
                        Spacer()

                        Image(systemName: "eye.slash")
                    }
                    .padding(.top, 2)
                }
                .padding()
                .background(
                    Image(.headerPattern)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(width: 400, height: 300,
                               alignment: .topTrailing)
                        .clipped()
                        .ignoresSafeArea(.all)
                )
                .foregroundStyle(.white)
                .ignoresSafeArea(.all)
                .frame(width: screenWidth, height: screenHeight * 0.20)
            }
            .background(Color.midnightBlue)

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
                    .padding(.vertical, 12)
                    .padding(.bottom, 6)
                    
                    Divider()
                    
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
                    .padding(.top, 12)
                    .padding(.bottom)
                }
            }
            .padding()
            .frame(width: screenWidth)

            Spacer()
        }
        .navigationTitle("Send")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear(perform: setToolbarAppearence)
    }
}

#Preview {
    NavigationStack {
        SendFlowSetAmountScreen()
    }
}
