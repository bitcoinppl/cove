//
//  NewWalletSelectScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewWalletSelectScreen: View {
    @Environment(\.colorScheme) var colorScheme
    @State var showSelectDialog: Bool = false

    // private
    let routeFactory: RouteFactory = .init()

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

                Button(action: { showSelectDialog = true }) {
                    HStack {
                        Image(systemName: "externaldrive")
                            .font(.title2)
                        Text("On a Hardware Wallet")
                            .font(.headline)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 25)
                    .background(.green.opacity(colorScheme == .dark ? 0.85 : 1))
                    .foregroundColor(.white)
                    .cornerRadius(12)
                }
                .buttonStyle(PlainButtonStyle())
            }
            .padding(.horizontal)
            .confirmationDialog(
                "Import hardware wallet using",
                isPresented: $showSelectDialog,
                titleVisibility: .visible
            ) {
                NavigationLink(value: routeFactory.qrImport()) {
                    Text("QR Code")
                }
                NavigationLink(value: routeFactory.nfcImport()) {
                    Text("NFC coming soon...")
                }

                NavigationLink(value: routeFactory.fileImport()) {
                    Text("File coming soon...")
                }
            }

            Spacer()
            Spacer()
        }
        .navigationBarTitleDisplayMode(.inline)
    }

    private func walletOptionButton(
        title: String, icon: String, color: Color, destination: some Hashable
    ) -> some View {
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
