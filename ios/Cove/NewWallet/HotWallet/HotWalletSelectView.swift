//
//  HotWalletSelectView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct HotWalletSelectView: View {
    @State private var isSheetShown = false

    var body: some View {
        VStack(spacing: 20) {
            Text("Select Wallet Option")
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top)

            Spacer()

            Button(action: { isSheetShown = true }) {
                HStack {
                    Image(systemName: "plus.circle.fill")
                    Text("Create Wallet")
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 25)
                .background(Color.blue)
                .foregroundColor(.white)
                .cornerRadius(10)
            }
            .confirmationDialog("Select Number of Words", isPresented: $isSheetShown) {
                NavigationLink(value: HotWalletRoute.create(words: NumberOfBip39Words.twelve).intoRoute()) {
                    Text("12 Words")
                }
                NavigationLink(value: HotWalletRoute.create(words: NumberOfBip39Words.twentyFour).intoRoute()) {
                    Text("24 Words")
                }
            }

            NavigationLink(value: HotWalletRoute.import.intoRoute()) {
                HStack {
                    Image(systemName: "arrow.down.circle.fill")
                    Text("Restore Wallet")
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 25)
                .background(Color.secondary.opacity(0.1))
                .foregroundColor(.primary)
                .cornerRadius(10)
            }

            Spacer()
            Spacer()
        }
        .padding()
        .navigationBarTitleDisplayMode(.inline)
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    HotWalletSelectView()
}
