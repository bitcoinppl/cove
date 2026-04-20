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
    @Environment(\.navigate) private var navigate
    @State private var walletList: [WalletMetadata] = []
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
                    ForEach(walletList, id: \.id) { wallet in
                        Button(action: {
                            guard draggedWalletId == nil else { return }
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
                        .onDrag {
                            draggedWalletId = wallet.id
                            UIImpactFeedbackGenerator(style: .light).impactOccurred()
                            return NSItemProvider(object: "\(wallet.id)" as NSString)
                        }
                        .onDrop(
                            of: [UTType.plainText],
                            delegate: SidebarWalletDropDelegate(
                                item: wallet,
                                wallets: $walletList,
                                draggedWalletId: $draggedWalletId,
                                onReorderCommitted: persistWalletOrder
                            )
                        )
                    }
                }

                Spacer()

                VStack(spacing: 32) {
                    Divider()
                        .overlay(.coveLightGray)
                        .opacity(0.50)

                    Button(action: { goTo(RouteFactory().newWalletSelect()) }) {
                        HStack(spacing: 20) {
                            Image(systemName: "wallet.bifold")
                            Text("Add Wallet")
                                .font(.callout)
                            Spacer()
                        }
                        .foregroundColor(.white)
                        .contentShape(Rectangle())
                    }

                    Button(action: { goTo(Route.settings(.main)) }) {
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
        .onAppear {
            walletList = app.wallets
        }
        .onChange(of: app.wallets) { _, updated in
            walletList = updated
        }
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

    private func persistWalletOrder() {
        do {
            try app.database.wallets().reorderWallets(orderedIds: walletList.map(\.id))
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        } catch {
            Log.error("Failed to reorder wallets \(error)")
            walletList = app.wallets
        }
    }
}

private struct SidebarWalletDropDelegate: DropDelegate {
    let item: WalletMetadata
    @Binding var wallets: [WalletMetadata]
    @Binding var draggedWalletId: WalletId?
    let onReorderCommitted: () -> Void

    func dropEntered(info _: DropInfo) {
        guard let draggedWalletId,
              draggedWalletId != item.id,
              let from = wallets.firstIndex(where: { $0.id == draggedWalletId }),
              let to = wallets.firstIndex(where: { $0.id == item.id })
        else { return }

        withAnimation(.spring(response: 0.25, dampingFraction: 0.85)) {
            wallets.move(
                fromOffsets: IndexSet(integer: from),
                toOffset: to > from ? to + 1 : to
            )
        }
    }

    func performDrop(info _: DropInfo) -> Bool {
        guard draggedWalletId != nil else { return false }
        draggedWalletId = nil
        onReorderCommitted()
        return true
    }
}
