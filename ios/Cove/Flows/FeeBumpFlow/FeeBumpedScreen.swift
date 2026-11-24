//
//  FeeBumpedScreen.swift
//  Cove
//
//  Created by Praveen Perera on 11/23/26.
//

import SwiftUI

/// Shown once a fee bump's replacement is broadcast, in place of the send flow's sent alert
///
/// The confirm screen swaps to it rather than pushing it, see `FeeBumpBroadcast`
struct FeeBumpedScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(\.colorScheme) private var colorScheme

    let broadcast: FeeBumpBroadcast
    let manager: WalletManager

    private var fiat: FeeBumpFiat {
        FeeBumpFiat(app: app, manager: manager)
    }

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            HeaderIcon(isSent: true, isConfirmed: true, numberOfConfirmations: 3)
                .padding(.bottom, 24)

            VStack(spacing: 6) {
                Text("Sped Up")
                    .font(.title)
                    .fontWeight(.semibold)
                    .accessibilityIdentifier("spedUp.title")

                Text("Rebroadcast with a higher fee.")
                    .foregroundStyle(.secondary)

                if let estimate = broadcast.confirmationEstimate() {
                    Text(estimate)
                        .fontWeight(.semibold)
                        .foregroundStyle(.secondary)
                }
            }
            .font(.subheadline)
            .multilineTextAlignment(.center)

            VStack(spacing: 4) {
                Text("\(broadcast.fee.satsString()) sats")
                    .font(.largeTitle)
                    .fontWeight(.bold)
                    .accessibilityIdentifier("spedUp.fee")

                if let fiatAmount = fiat.approximate(broadcast.fee) {
                    Text(fiatAmount)
                        .font(.subheadline)
                }

                Text("New network fee")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.top, 4)
            }

            Spacer()
            Spacer()

            Button("Done", action: done)
                .buttonStyle(DarkButtonStyle())
                .accessibilityIdentifier("spedUp.done")
        }
        .padding(.horizontal)
        .padding(.bottom)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(FeeBumpedBackground(colorScheme: colorScheme))
        .navigationBarBackButtonHidden()
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("spedUp.screen")
    }

    private func done() {
        app.loadAndReset(to: .selectedWallet(manager.id))
    }
}

private struct FeeBumpedBackground: View {
    let colorScheme: ColorScheme

    var body: some View {
        ZStack {
            Color.coveBg

            Image(.transactionDetailsPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .opacity(colorScheme == .light ? 0.40 : 1)
        }
        .ignoresSafeArea()
    }
}

/// Formats fee amounts in the user's fiat currency for the fee bump screens
struct FeeBumpFiat {
    let app: AppManager
    let manager: WalletManager

    /// The amount in fiat, `nil` until prices load
    func callAsFunction(_ amount: Amount) -> String? {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return nil
        }

        return manager.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    /// The amount in fiat as an approximation, such as "≈ $1.07 USD"
    func approximate(_ amount: Amount) -> String? {
        self(amount).map { "≈ \($0)" }
    }
}
