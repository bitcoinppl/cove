//
//  VerificationCompleteScreen.swift
//  Cove
//
//  Created by Praveen Perera on 12/4/24.
//

import Foundation
import SwiftUI

struct VerificationCompleteScreen: View {
    @Environment(AppManager.self) var app

    /// args
    let manager: WalletManager

    var body: some View {
        VStack(spacing: 24) {
            Spacer()
            Spacer()

            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: screenWidth * 0.46))
                .fontWeight(.light)
                .symbolRenderingMode(.palette)
                .foregroundStyle(.midnightBlue, Color.lightGreen)

            Spacer()
            Spacer()
            Spacer()

            HStack {
                DotMenuView(selected: 3, size: 5)
                Spacer()
            }

            VStack(spacing: 12) {
                HStack {
                    Text("You're all set!")
                        .font(.system(size: 38, weight: .semibold))
                        .foregroundStyle(.white)

                    Spacer()
                }

                HStack {
                    Text(
                        "All set! You’ve successfully verified your recovery words and can now access your wallet."
                    )
                    .font(.footnote)
                    .foregroundStyle(.coveLightGray.opacity(0.75))
                    .fixedSize(horizontal: false, vertical: true)

                    Spacer()
                }
            }

            Divider().overlay(Color.coveLightGray.opacity(0.50))

            Button("Go To Wallet") {
                do {
                    try manager.rust.markWalletAsVerified()
                    app.resetRoute(to: Route.selectedWallet(manager.id))
                } catch {
                    Log.error("Error marking wallet as verified: \(error)")
                }
            }
            .buttonStyle(PrimaryButtonStyle())
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(
            Image(.newWalletPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: screenHeight * 0.75, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .opacity(0.75)
        )
        .background(Color.midnightBlue)
    }
}

#Preview {
    AsyncPreview {
        VerificationCompleteScreen(manager: WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
    }
}
