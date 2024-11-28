//
//  NfcHelpView.swift
//  Cove
//
//  Created by Praveen Perera on 10/21/24.
//
import Foundation
import SwiftUI

struct NfcHelpView: View {
    var body: some View {
        Text("How do I import using NFC?")
            .font(.title)
            .fontWeight(.bold)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 12)
            .frame(alignment: .center)
            .padding(.vertical, 18)

        ScrollView {
            VStack(alignment: .leading, spacing: 32) {
                VStack(alignment: .leading, spacing: 12) {
                    Text("ColdCard Q1")
                        .font(.title2)
                        .fontWeight(.bold)

                    Text("1. Enable NFC by going to 'Settings' > 'Hardware On/Off' > 'NFC Sharing' ")
                    Text("2. Go to 'Advanced / Tools'")
                    Text("3. Export Wallet > 'Descriptor' > 'Segwit P2WPKH'")
                    Text("4. Press the 'Enter' button, then the 'NFC' button")
                    Text("5. Bring the phone to the top of the screen")
                }

                Divider()

                VStack(alignment: .leading, spacing: 12) {
                    Text("ColdCard MK4")
                        .font(.title2)
                        .fontWeight(.bold)

                    Text("1. Enable NFC by going to 'Settings' > 'Hardware On/Off' > 'NFC Sharing' ")
                    Text("2. Go to 'Advanced / Tools'")
                    Text("3. Export Wallet > 'Descriptor' > 'Segwit P2WPKH'")
                    Text("4. Press the 'Enter' button, then the 'NFC' button")
                    Text("5. Bring the phone to the to the coldcard near the 8 button")
                }
            }
        }
        .padding(22)
    }
}

#Preview {
    NfcHelpView()
}
