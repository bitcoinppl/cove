//
//  SelectedWalletContainer.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SelectedWalletContainer: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(AppManager.self) private var app

    let id: WalletId

    @State private var manager: WalletManager?

    private var iOS26OrLater: Bool {
        if #available(iOS 26.0, *) {
            return true
        }
        return false
    }

    var body: some View {
        Group {
            if let manager {
                SelectedWalletScreen(manager: manager)
                    .background(
                        iOS26OrLater
                            ? nil
                            : manager.loadState == .loading
                            ? LinearGradient(
                                colors: [
                                    .black.opacity(colorScheme == .dark ? 0.9 : 0),
                                    .black.opacity(colorScheme == .dark ? 0.9 : 0),
                                ], startPoint: .top, endPoint: .bottom
                            )
                            : LinearGradient(
                                stops: [
                                    .init(
                                        color: .midnightBlue,
                                        location: 0.20
                                    ),
                                    .init(
                                        color: colorScheme == .dark ? .black.opacity(0.9) : .white,
                                        location: 0.20
                                    ),
                                ], startPoint: .top, endPoint: .bottom
                            )
                    )
                    .background(iOS26OrLater ? nil : Color.white)
            } else {
                FullPageLoadingView(title: "Loading wallet...")
            }
        }
        .task(id: id) {
            if manager?.id != id {
                manager = nil
            }

            let generation = app.captureLoadAndResetGeneration()

            do {
                switch try await app.prepareSelectedWallet(id: id, generation: generation) {
                case let .ready(manager):
                    self.manager = manager

                    do {
                        try await manager.startWalletScan()
                    } catch {
                        Log.error("Wallet Scan Failed \(error.localizedDescription)")
                    }
                case .redirected:
                    return
                }
            } catch is CancellationError {
                return
            } catch {
                Log.error("Unable to prepare selected wallet \(id): \(error)")
            }
        }
    }
}

#Preview {
    SelectedWalletContainer(id: WalletId())
        .environment(AppManager.shared)
}
