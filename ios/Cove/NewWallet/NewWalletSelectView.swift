//
//  NewWalletSelect.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewWalletSelect: View {
    var body: some View {
        VStack {
            HStack {
                Text("How do you want to secure your Bitcoin?")
                    .font(.largeTitle)
                    .multilineTextAlignment(.center)
            }
            .padding(.top, 20)
            .padding(.bottom, 20)
            .padding(.horizontal, 30)

            Spacer()

            NavigationLink(value: RouteFactory().newHotWallet()) {
                Spacer()
                Text("On This Device").font(.title)
                Spacer()
            }
            .cornerRadius(2.0)
            .frame(maxHeight: .infinity)
            .background(
                RoundedRectangle(cornerRadius: 15)
                    .fill(LinearGradient(colors: [
                            Color.blue, Color.blue.opacity(0.8), Color.blue.opacity(0.6),
                        ],
                        startPoint: .topLeading, endPoint: .bottomTrailing)
                    )
            )
            .padding(.vertical, 30)
            .padding(.horizontal, 40)
            .foregroundColor(.white)

            Spacer()

            NavigationLink(value: RouteFactory().newColdWallet()) {
                Spacer()
                Text("On Hardware Wallet").font(.title)
                Spacer()
            }
            .frame(maxHeight: .infinity)
            .background(
                RoundedRectangle(cornerRadius: 15)
                    .fill(LinearGradient(colors: [
                            Color.green, Color.green.opacity(0.8), Color.green.opacity(0.6),
                        ],
                        startPoint: .topLeading, endPoint: .bottomTrailing)
                    )
            )
            .foregroundColor(.white)
            .padding(.vertical, 30)
            .padding(.horizontal, 40)
            Spacer()
        }
        .enableInjection()
    }

    #if DEBUG
    @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    NewWalletSelect()
}
