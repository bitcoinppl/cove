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
        VStack {
            Button(action: { isSheetShown = true }) {
                Text("Create Wallet")
                    .font(.title3)
                    .bold()
                    .foregroundStyle(.white)
                    .frame(minWidth: 250, minHeight: 90)
                    .confirmationDialog("Background Color", isPresented: $isSheetShown) {
                        NavigationLink(value: HotWalletRoute.create(words: NumberOfBip39Words.twelve).intoRoute()) {
                            Text("12 Words")
                        }
                        NavigationLink(value: HotWalletRoute.create(words: NumberOfBip39Words.twentyFour).intoRoute()) {
                            Text("24 Words")
                        }
                    }
            }
            .background(
                RoundedRectangle(cornerRadius: 15)
                    .fill(.green)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 15)
                    .stroke(Color.green, lineWidth: 2)
                    .brightness(-0.1)
            )
            .padding(.vertical, 15)

            NavigationLink(value: HotWalletRoute.import.intoRoute()) {
                Text("Restore Wallet")
                    .font(.title3)
                    .bold()
                    .foregroundStyle(.black)
                    .frame(minWidth: 250, minHeight: 90)
            }
            .overlay(
                RoundedRectangle(cornerRadius: 15)
                    .stroke(Color.black, lineWidth: 2)
            )
            .padding(.vertical, 15)
        }
    }
}

#Preview {
    HotWalletSelectView()
}
