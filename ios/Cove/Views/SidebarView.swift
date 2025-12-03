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
                    ForEach(app.wallets, id: \.id) { wallet in
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
                        .background(Color.coveLightGray.opacity(0.06))
                        .cornerRadius(10)
                        .contentShape(
                            .contextMenuPreview,
                            RoundedRectangle(cornerRadius: 10)
                        )
                        .contextMenu {
                            Button("Settings") {
                                app.isSidebarVisible = false
                                app.pushRoutes(RouteFactory().nestedWalletSettings(id: wallet.id))
                            }
                        }
                    }
                }

                Spacer()

                VStack(spacing: 32) {
                    Divider()
                        .overlay(.coveLightGray)
                        .opacity(0.50)

                    HStack {
                        Button(action: { goTo(RouteFactory().newWalletSelect()) }) {
                            HStack(spacing: 20) {
                                Image(systemName: "wallet.bifold")
                                Text("Add Wallet")
                                    .font(.callout)
                            }
                            .foregroundColor(.white)
                        }

                        Spacer()
                    }

                    HStack {
                        Button(action: { goTo(Route.settings(.main)) }) {
                            HStack(spacing: 22) {
                                Image(systemName: "gear")
                                Text("Settings")
                                    .font(.callout)
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
        withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
            app.isSidebarVisible = false
        }

        Task {
            try? await Task.sleep(for: .milliseconds(200))
            await navigateRoute(route)
        }
    }

    private func navigateRouteOnMain(_ route: Route) {
        navigate(route)
    }

    private func navigateRoute(_ route: Route) async {
        do {
            if case let Route.selectedWallet(id: id) = route {
                try app.rust.selectWallet(id: id)
                return
            }

            if !app.hasWallets, route == Route.newWallet(.select) {
                app.resetRoute(to: [RouteFactory().newWalletSelect()])
                return
            }
        } catch {
            Log.error("Failed to select wallet \(error)")
        }

        navigateRouteOnMain(route)
    }

    private func loadWallets() {
        do {
            self.wallets = try Database().wallets().all()
        } catch {
            Log.error("Failed to get wallets \(error)")
        }
    }
}
