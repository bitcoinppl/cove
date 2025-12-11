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
    @Environment(\.navigate) private var navigate

    let id: WalletId
    @State private var manager: WalletManager? = nil

    func loadManager() {
        if manager != nil, app.walletManager == nil { return }

        do {
            Log.debug("Getting wallet \(id)")
            manager = try app.getWalletManager(id: id)
        } catch {
            Log.error("Something went very wrong: \(error)")
            do {
                let wallets = try Database().wallets().all()
                let wallet = wallets.first(where: { $0.id != id })

                if let wallet {
                    try app.rust.selectWallet(id: wallet.id)
                } else {
                    app.loadAndReset(to: Route.newWallet(.select))
                }
            } catch {
                app.loadAndReset(to: Route.newWallet(.select))
            }
        }
    }

    var body: some View {
        Group {
            if let manager {
                SelectedWalletScreen(manager: manager)
                    .background(
                        manager.loadState == .loading
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
                    .background(Color.white)

            } else {
                Text("Loading...")
            }
        }
        .onAppear(perform: loadManager)
        .task {
            // start scan immediately (sends cached data first, then scans)
            if let manager {
                do {
                    try await manager.rust.startWalletScan()
                } catch {
                    Log.error("Wallet Scan Failed \(error.localizedDescription)")
                }
            }
        }
        .onDisappear {
            manager?.dispatch(.selectedWalletDisappeared)
        }
        .task(id: manager?.loadState) {
            guard case .loaded = manager?.loadState, let manager else { return }
            app.updateWalletVm(manager)
        }
    }
}

#Preview {
    SelectedWalletContainer(id: WalletId())
        .environment(AppManager.shared)
}
