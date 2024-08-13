//
//  NewWalletSelectScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewWalletSelectScreen: View {
    @Environment(\.colorScheme) var colorScheme

    var body: some View {
        VStack(spacing: 30) {
            Text("How do you want to secure your Bitcoin?")
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top)

            Spacer()

            VStack(spacing: 30) {
                walletOptionButton(
                    title: "On This Device",
                    icon: "iphone",
                    color: .blue,
                    destination: RouteFactory().newHotWallet()
                )

                walletOptionButton(
                    title: "On Hardware Wallet",
                    icon: "externaldrive",
                    color: .green,
                    destination: RouteFactory().newColdWallet()
                )
            }
            .padding(.horizontal)

            Spacer()
            Spacer()
        }
        .navigationBarTitleDisplayMode(.inline)
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif

    private func walletOptionButton(title: String, icon: String, color: Color, destination: some Hashable) -> some View {
        NavigationLink(value: destination) {
            HStack {
                Image(systemName: icon)
                    .font(.title2)
                Text(title)
                    .font(.headline)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 25)
            .background(color.opacity(colorScheme == .dark ? 0.85 : 1))
            .foregroundColor(.white)
            .cornerRadius(12)
        }
        .buttonStyle(PlainButtonStyle())
    }
}

#Preview {
    NewWalletSelectScreen()
}
