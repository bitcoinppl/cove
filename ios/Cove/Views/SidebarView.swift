//
//  SidebarView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SidebarView: View {
    @Environment(AppManager.self) private var app

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

                    Button(action: app.closeSidebarAndScanNfc) {
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
                            app.closeSidebarAndSelectWallet(wallet.id)
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
                                app.closeSidebarAndOpenWalletSettings(wallet.id)
                            }
                        }
                    }
                }

                Spacer()

                VStack(spacing: 32) {
                    Divider()
                        .overlay(.coveLightGray)
                        .opacity(0.50)

                    Button(action: app.closeSidebarAndOpenNewWallet) {
                        HStack(spacing: 20) {
                            Image(systemName: "wallet.bifold")
                            Text("Add Wallet")
                                .font(.callout)
                            Spacer()
                        }
                        .foregroundColor(.white)
                        .contentShape(Rectangle())
                    }

                    Button(action: app.closeSidebarAndOpenSettings) {
                        HStack(spacing: 22) {
                            Image(systemName: "gear")
                            Text("Settings")
                                .font(.callout)
                            Spacer()
                        }
                        .foregroundColor(.white)
                        .contentShape(Rectangle())
                    }
                }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity)
        .background(.midnightBlue)
    }
}
