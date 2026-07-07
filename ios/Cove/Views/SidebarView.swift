//
//  SidebarView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI
import UniformTypeIdentifiers

struct SidebarView: View {
    @Environment(AppManager.self) private var app
    @State private var localWallets: [WalletMetadata] = []
    @State private var draggedWalletId: WalletId?

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

            ScrollView {
                LazyVStack(spacing: 12) {
                    ForEach(localWallets, id: \.id) { wallet in
                        walletRow(wallet)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .top)
            }
            .scrollIndicators(.hidden)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .onDrop(
                of: [.text],
                delegate: SidebarWalletListDropDelegate(
                    wallets: $localWallets,
                    draggedWalletId: $draggedWalletId,
                    persistOrder: persistWalletOrder
                )
            )
            .onAppear {
                localWallets = app.wallets
            }
            .onChange(of: app.wallets) { _, wallets in
                guard draggedWalletId == nil else { return }

                localWallets = wallets
            }

            SidebarFooter(
                addWallet: app.closeSidebarAndOpenNewWallet,
                openSettings: app.closeSidebarAndOpenSettings
            )
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(.midnightBlue)
    }

    @ViewBuilder
    private func walletRow(_ wallet: WalletMetadata) -> some View {
        if localWallets.count > 1 {
            walletButton(wallet)
                .opacity(draggedWalletId == wallet.id ? 0.72 : 1)
                .onDrag {
                    draggedWalletId = wallet.id

                    return NSItemProvider(object: wallet.id as NSString)
                }
                .onDrop(
                    of: [.text],
                    delegate: SidebarWalletDropDelegate(
                        wallet: wallet,
                        wallets: $localWallets,
                        draggedWalletId: $draggedWalletId,
                        persistOrder: persistWalletOrder
                    )
                )
        } else {
            walletButton(wallet)
        }
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
            .padding()
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.coveLightGray.opacity(0.06))
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func persistWalletOrder(_ wallets: [WalletMetadata]) {
        app.reorderWallets(walletIds: wallets.map(\.id))
    }
}

private struct SidebarWalletDropDelegate: DropDelegate {
    let wallet: WalletMetadata
    @Binding var wallets: [WalletMetadata]
    @Binding var draggedWalletId: WalletId?
    let persistOrder: ([WalletMetadata]) -> Void

    func validateDrop(info _: DropInfo) -> Bool {
        draggedWalletId != nil
    }

    func dropEntered(info _: DropInfo) {
        guard
            let draggedWalletId,
            draggedWalletId != wallet.id,
            let sourceIndex = wallets.firstIndex(where: { $0.id == draggedWalletId }),
            let destinationIndex = wallets.firstIndex(where: { $0.id == wallet.id })
        else {
            return
        }

        withAnimation(.spring(response: 0.20, dampingFraction: 0.82)) {
            let movedWallet = wallets.remove(at: sourceIndex)
            wallets.insert(movedWallet, at: destinationIndex)
        }
    }

    func dropUpdated(info _: DropInfo) -> DropProposal? {
        DropProposal(operation: .move)
    }

    func performDrop(info _: DropInfo) -> Bool {
        persistOrder(wallets)
        draggedWalletId = nil

        return true
    }
}

private struct SidebarWalletListDropDelegate: DropDelegate {
    @Binding var wallets: [WalletMetadata]
    @Binding var draggedWalletId: WalletId?
    let persistOrder: ([WalletMetadata]) -> Void

    func validateDrop(info _: DropInfo) -> Bool {
        draggedWalletId != nil
    }

    func dropUpdated(info _: DropInfo) -> DropProposal? {
        DropProposal(operation: .move)
    }

    func performDrop(info _: DropInfo) -> Bool {
        persistOrder(wallets)
        draggedWalletId = nil

        return true
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
