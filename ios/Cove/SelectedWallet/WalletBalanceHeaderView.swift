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
    let metadata: WalletMetadata
    let updater: (WalletViewModelAction) -> ()
    
    var balanceString: String {
        if !metadata.sensitiveVisible {
            return "************"
        }
        
        return switch metadata.selectedUnit {
        case .btc: balance.btcString()
        case .sat: balance.satsString()
        }
    }
    
    var eyeIcon: String {
        metadata.sensitiveVisible ? "eye" : "eye.slash"
    }
    
    var fontSize: CGFloat {
        let btc = balance.asBtc()
        
        // Base font size
        let baseFontSize: CGFloat = 34
            
        // Calculate the number of digits
        let digits = btc > 0 ? Int(log10(btc)) + 1 : 1
            
        // Reduce font size by 2 for each additional digit beyond 1
        let fontSizeReduction = CGFloat(max(0, (digits - 1) * 2))
            
        // Ensure minimum font size of 20
        return max(baseFontSize - fontSizeReduction, 20)
    }
    
    var body: some View {
        VStack {
            HStack {
                Picker("Currency",
                       selection: Binding(get: { metadata.selectedUnit },
                                          set: { updater(.updateUnit($0)) }))
                {
                    Text(String(Unit.btc)).tag(Unit.btc)
                    Text(String(Unit.sat)).tag(Unit.sat)
                }
                .pickerStyle(SegmentedPickerStyle())
                .frame(width: 120)
                
                Spacer()
                
                Image(systemName: eyeIcon)
                    .foregroundColor(.gray)
                    .onTapGesture {
                        updater(.toggleSensitiveVisibility)
                    }
            }
            
            HStack {
                Text("Your Balance")
                    .foregroundColor(.gray)
                    .font(.subheadline)
                    .padding(.leading, 2)
                
                Spacer()
            }
            
            Text(balanceString)
                .font(.system(size: fontSize, weight: .bold))
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

#Preview("btc") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true
    
    return
        WalletBalanceHeaderView(balance:
            Amount.fromSat(sats: 1_000_738),
            metadata: metadata,
            updater: { _ in () })
        .padding()
}

#Preview("sats") {
    var metadata = walletMetadataPreview()
    metadata.selectedUnit = .sat
    metadata.sensitiveVisible = true

    return
        WalletBalanceHeaderView(balance:
            Amount.fromSat(sats: 1_000_738),
            metadata: metadata,
            updater: { _ in () })
        .padding()
}

#Preview("hidden") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = false
    
    return
        WalletBalanceHeaderView(balance:
            Amount.fromSat(sats: 1_000_738),
            metadata: metadata,
            updater: { _ in () })
        .padding()
}

#Preview("lots of btc") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true
    
    return
        WalletBalanceHeaderView(balance:
            Amount.fromSat(sats: 10_000_000_738),
            metadata: metadata,
            updater: { _ in () })
        .padding()
}
