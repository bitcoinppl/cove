//
//  SidebarView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SidebarView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate

    let currentRoute: Route
    let wallets: [WalletMetadata]

    init(currentRoute: Route, wallets: [WalletMetadata]? = nil) {
        self.currentRoute = currentRoute
        if let wallets {
            self.wallets = wallets
            return
        }

        do {
            self.wallets = try Database().wallets().all()
            Log.debug("wallets: \(self.wallets)")
        } catch {
            Log.error("Failed to get wallets \(error)")
            self.wallets = []
        }
    }

    func setForeground(_ route: Route) -> LinearGradient {
        if RouteFactory().isSameParentRoute(route: route, routeToCheck: currentRoute) {
            LinearGradient(
                colors: [
                    Color.blue,
                    Color.blue.opacity(0.9),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        } else {
            LinearGradient(
                colors: [
                    Color.primary.opacity(0.8), Color.primary.opacity(0.7),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        }
    }

    var body: some View {
        HStack(alignment: .top) {
            VStack {
                HStack(alignment: .top) {
                    Image(.icon)
                        .resizable()
                        .frame(width: 65, height: 65)
                        .clipShape(Circle())

                    Spacer()

                    Button(action: app.nfcReader.scan) {
                        Image(systemName: "wave.3.right")
                    }
                    .foregroundStyle(.white)
                }

                Divider()
                    .overlay(Color(.white))
                    .opacity(0.50)
                    .padding(.vertical, 22)

                HStack {
                    Text("My Wallets")
                        .font(.subheadline)
                        .fontWeight(.medium)
                        .foregroundStyle(.white)

                    Spacer()
                }
                .padding(.bottom, 16)

                VStack(spacing: 12) {
                    ForEach(wallets, id: \.id) { wallet in
                        Button(action: {
                            goTo(Route.selectedWallet(wallet.id))
                        }) {
                            HStack(spacing: 10) {
                                Circle()
                                    .fill(Color(wallet.color))
                                    .frame(width: 8, height: 8, alignment: .leading)

                                Text(wallet.name)
                                    .font(.footnote)
                                    .fontWeight(.medium)
                                    .foregroundStyle(.white)
                                    .lineLimit(1)
                                    .minimumScaleFactor(0.80)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .frame(maxWidth: .infinity)
                        }
                        .padding()
                        .background(Color.lightGray.opacity(0.06))
                        .cornerRadius(10)
                        .contentShape(
                            .contextMenuPreview,
                            RoundedRectangle(cornerRadius: 10)
                        )
                        .contextMenu {
                            Button("Settings") {
                                app.isSidebarVisible = false

                                do {
                                    try app.rust.selectWallet(
                                        id: wallet.id,
                                        nextRoute: Route.walletSettings(wallet.id)
                                    )
                                } catch {
                                    Log.error("Failed to select wallet \(error)")
                                    goTo(Route.selectedWallet(wallet.id))
                                }
                            }
                        }
                    }
                }

                Spacer()

                VStack(spacing: 32) {
                    Divider()
                        .overlay(.lightGray)
                        .opacity(0.50)

                    HStack {
                        Button(action: { goTo(RouteFactory().newWalletSelect()) }) {
                            HStack(spacing: 20) {
                                Image(systemName: "wallet.bifold")
                                Text("Add Wallet")
                            }
                            .foregroundColor(.white)
                        }

                        Spacer()
                    }

                    HStack {
                        Button(action: { goTo(Route.settings) }) {
                            HStack(spacing: 22) {
                                Image(systemName: "gear")
                                Text("Settings")
                            }
                            .foregroundColor(.white)
                        }

                        Spacer()
                    }
                }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity)
        .background(.midnightBlue)
    }

    func goTo(_ route: Route) {
        app.isSidebarVisible = false

        Task {
            try? await Task.sleep(for: .milliseconds(300))

            if case let Route.selectedWallet(id: id) = route {
                let selected: ()? = try? app.rust.selectWallet(id: id)
                if selected == nil { app.loadAndReset(to: route) }
                return
            }

            if !app.hasWallets, route == Route.newWallet(.select) {
                return app.resetRoute(to: [RouteFactory().newWalletSelect()])
            }

            navigate(route)
        }
    }
}

#if DEBUG
    #Preview {
        HStack {
            SidebarView(
                currentRoute: Route.listWallets,
                wallets: [
                    WalletMetadata("Test Wallet", preview: true),
                    WalletMetadata("Second Wallet", preview: true),
                    WalletMetadata("Coldcard Q1", preview: true),
                ]
            )
            .environment(AppManager())
            .background(Color.white)
            .frame(width: 280)

            Spacer()
        }
    }
#endif
