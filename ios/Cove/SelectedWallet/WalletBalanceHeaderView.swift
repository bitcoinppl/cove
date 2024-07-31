//
//  BalanceHeaderView.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import SwiftUI

struct WalletBalanceHeaderView: View {
    // confirmed balance
    let balance: Amount
    
    @State private var selectedUnits = Unit.btc
    
    var balanceString: String {
        balance.btcString()
    }
    
    var body: some View {
        VStack {
            HStack {
                Picker("Currency", selection: $selectedUnits) {
                    Text(String(Unit.btc)).tag(Unit.btc)
                    Text(String(Unit.sat)).tag(Unit.sat)
                }
                .pickerStyle(SegmentedPickerStyle())
                .frame(width: 120)
                
                Spacer()
                
                Image(systemName: "eye")
                    .foregroundColor(.gray)
            }
            
            HStack {
                Text("Your Balance")
                    .foregroundColor(.gray)
                    .font(.subheadline)
                    .padding(.leading, 2)
                
                Spacer()
            }
            
            Text(balanceString)
                .font(.system(size: 38, weight: .bold))
                .padding(.top, 16)
                .padding(.bottom, 32)
            
            HStack(spacing: 16) {
                Button(action: {
                    // Receive action
                }) {
                    HStack(spacing: 10) {
                        Image(systemName: "arrow.down.left")
                        
                        Text("Receive")
                    }
                    .foregroundColor(.white)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(Color.blue)
                    .cornerRadius(8)
                }
                
                Button(action: {
                    // Send action
                }) {
                    HStack(spacing: 10) {
                        Image(systemName: "arrow.up.right")
                        
                        Text("Send")
                    }
                    .foregroundColor(.blue)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(Color.white)
                    .cornerRadius(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(Color.blue, lineWidth: 1)
                    )
                }
            }
        }
        .padding()
        .background(Color(UIColor.systemGray6))
        .cornerRadius(16)
    }
}

#Preview {
    WalletBalanceHeaderView(balance: Amount.fromSat(sats: 1_000_738))
        .padding()
}
