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
        VStack(spacing: 0) {
            SidebarHeader(scanNfc: app.closeSidebarAndScanNfc)

            List {
                ForEach(app.wallets, id: \.id) { wallet in
                    walletButton(wallet)
                        .padding(.bottom, 12)
                        .listRowBackground(Color.clear)
                        .listRowInsets(EdgeInsets())
                        .listRowSeparator(.hidden)
                }
                .onMove(perform: app.moveWallets)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .scrollIndicators(.hidden)
            .background(Color.clear)
            .environment(\.editMode, .constant(.active))

            SidebarFooter(
                addWallet: app.closeSidebarAndOpenNewWallet,
                openSettings: app.closeSidebarAndOpenSettings
            )
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(.midnightBlue)
    }

    private func walletButton(_ wallet: WalletMetadata) -> some View {
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

private struct SidebarHeader: View {
    let scanNfc: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .top) {
                Image(.icon)
                    .resizable()
                    .frame(width: 65, height: 65)
                    .clipShape(Circle())

                Spacer()

                Button(action: scanNfc) {
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
        }
    }
}

private struct SidebarFooter: View {
    let addWallet: () -> Void
    let openSettings: () -> Void

    var body: some View {
        VStack(spacing: 32) {
            Divider()
                .overlay(.coveLightGray)
                .opacity(0.50)

            Button(action: addWallet) {
                sidebarActionLabel(
                    title: "Add Wallet",
                    systemImage: "wallet.bifold",
                    spacing: 20
                )
            }

            Button(action: openSettings) {
                sidebarActionLabel(
                    title: "Settings",
                    systemImage: "gear",
                    spacing: 22
                )
            }
        }
        .padding(.top, 20)
    }

    private func sidebarActionLabel(title: String, systemImage: String, spacing: CGFloat) -> some View {
        HStack(spacing: spacing) {
            Image(systemName: systemImage)
            Text(title)
                .font(.callout)
            Spacer()
        }
        .foregroundColor(.white)
        .contentShape(Rectangle())
    }
}

#if DEBUG
    #Preview("Many Wallets") {
        let app = {
            let app = AppManager.shared
            app.wallets = (1 ... 14).map { WalletMetadata("Wallet \($0)", preview: true) }
            return app
        }()

        SidebarView(currentRoute: app.currentRoute)
            .frame(width: 280)
            .environment(app)
    }
#endif
