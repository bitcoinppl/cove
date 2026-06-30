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

    private var iOS26OrLater: Bool {
        if #available(iOS 26.0, *) { return true }
        return false
    }

    var body: some View {
        WalletManagerHost(
            walletId: id,
            loading: {
                if let metadata = app.walletMetadata(id: id) {
                    SelectedWalletLoadingScreen(metadata: metadata)
                } else {
                    FullPageLoadingView(title: String(localized: "Loading wallet..."))
                }
            },
            onError: handleManagerError
        ) { manager in
            SelectedWalletScreen(manager: manager)
                .background(
                    iOS26OrLater
                        ? nil
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
                .task {
                    // start scan immediately (sends cached data first, then scans)
                    do {
                        try await manager.rust.startWalletScan()
                    } catch {
                        Log.error("Wallet Scan Failed \(error.localizedDescription)")
                    }
                }
        }
    }

    private func handleManagerError(_ error: Error) {
        switch error {
        case let WalletManagerError.DatabaseCorruption(walletId, errorMessage):
            Log.error("Wallet database corrupted for \(walletId): \(errorMessage)")
            app.alertState = TaggedItem(
                .walletDatabaseCorrupted(walletId: walletId, error: errorMessage)
            )
        default:
            Log.error("Something went very wrong: \(error)")
            do {
                let wallets = try Database().wallets().all()
                let wallet = wallets.first(where: { $0.id != id })

                if let wallet {
                    try app.selectWalletOrThrow(wallet.id)
                } else {
                    app.loadAndReset(to: Route.newWallet(.select))
                }
            } catch {
                app.loadAndReset(to: Route.newWallet(.select))
            }
        }
    }
}

private struct SelectedWalletLoadingScreen: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(AppManager.self) private var app

    let metadata: WalletMetadata

    private let screenHeight = UIScreen.main.bounds.height
    private let navBarAndScrollInsets: CGFloat = 100

    private var iOS26OrLater: Bool {
        if #available(iOS 26.0, *) { return true }
        return false
    }

    private var toolbarTextColor: Color {
        .white
    }

    private var canGoBack: Bool {
        app.rust.canGoBack()
    }

    private var titleContent: some View {
        HStack(spacing: 10) {
            if case .cold = metadata.walletType {
                BitcoinShieldIcon(width: 13, color: toolbarTextColor)
            }

            Text(metadata.name)
                .foregroundStyle(toolbarTextColor)
                .font(.callout)
                .fontWeight(.semibold)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
        }
        .padding(.vertical, 20)
        .padding(.horizontal, 28)
    }

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .navigationBarLeading) {
            Button(action: {
                if canGoBack {
                    app.popRoute()
                } else {
                    withAnimation {
                        app.toggleSidebar()
                    }
                }
            }) {
                Image(systemName: canGoBack ? "chevron.left" : "line.horizontal.3")
                    .adaptiveToolbarItemStyle(isPastHeader: false)
                    .font(.callout)
            }
            .contentShape(Rectangle())
            .accessibilityLabel(
                Text(canGoBack ? String(localized: "Back") : String(localized: "Menu"))
            )
        }

        ToolbarItemGroup(placement: .navigationBarTrailing) {
            HStack(spacing: 5) {
                Button(action: {
                    app.sheetState = .init(.qr)
                }) {
                    Image(systemName: "qrcode")
                        .adaptiveToolbarItemStyle(isPastHeader: false)
                        .font(.callout)
                }
                .accessibilityLabel(Text(String(localized: "QR Code")))

                Button(action: {}) {
                    Image(systemName: "ellipsis.circle")
                        .adaptiveToolbarItemStyle(isPastHeader: false)
                        .font(.callout)
                }
                .disabled(true)
                .accessibilityLabel(Text(String(localized: "More")))
            }
        }
    }

    private var content: some View {
        VStack(spacing: 0) {
            WalletBalanceLoadingHeaderView(metadata: metadata)

            if !CloudBackupManager.shared.isConfigured {
                VerifyReminder(walletId: metadata.id, isVerified: metadata.verified)
            }

            TransactionsLoadingCardView()
                .background(Color.coveBg)
        }
        .background(Color.coveBg)
        .toolbar { toolbarContent }
        .navigationTitleView { titleContent }
        .adaptiveToolbarStyle(showNavBar: false, reduceTransparency: reduceTransparency)
    }

    var body: some View {
        ScrollView {
            content
                .background(
                    VStack(spacing: 0) {
                        Color.midnightBlue
                            .frame(height: screenHeight * 0.40 + 500)
                        Color.coveBg
                    }
                    .offset(y: -500)
                )
        }
        .contentMargins(
            .top, -(safeAreaInsets.top + navBarAndScrollInsets), for: .scrollContent
        )
        .modifier(ScrollViewBackgroundModifier(iOS26OrLater: iOS26OrLater))
        .scrollIndicators(.hidden)
        .modifier(HiddenTopScrollEdgeModifier())
        .modifier(OuterBackgroundModifier(iOS26OrLater: iOS26OrLater))
        .onAppear {
            app.isPastHeader = false
        }
        .onDisappear {
            app.isPastHeader = false
        }
    }
}

private struct HiddenTopScrollEdgeModifier: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.scrollEdgeEffectHidden(true, for: .top)
        } else {
            content
        }
    }
}

private struct WalletBalanceLoadingHeaderView: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets

    let metadata: WalletMetadata

    private var balancePresentation: BalancePresentation {
        balancePresentationProvisional()
    }

    private var eyeIcon: String {
        metadata.sensitiveVisible ? "eye" : "eye.slash"
    }

    private func balanceLoadingView(size: ControlSize = .regular, scale: CGFloat = 1) -> some View {
        Group {
            if metadata.sensitiveVisible {
                ProgressView()
                    .controlSize(size)
                    .scaleEffect(scale)
            } else {
                Text("••••••")
            }
        }
    }

    var body: some View {
        VStack(spacing: 28) {
            VStack(spacing: 6) {
                HStack {
                    balanceLoadingView(scale: 0.7)
                        .foregroundColor(.white.opacity(balancePresentation.secondaryOpacity))
                        .tint(.white.opacity(balancePresentation.secondaryOpacity))
                        .font(.footnote)
                        .padding(.leading, 2)

                    Spacer()
                }

                HStack {
                    balanceLoadingView(size: .large)
                        .foregroundStyle(.white.opacity(balancePresentation.primaryOpacity))
                        .tint(.white.opacity(balancePresentation.primaryOpacity))
                        .font(.system(size: 34, weight: .bold))

                    Spacer()

                    Image(systemName: eyeIcon)
                        .foregroundColor(.gray)
                }
            }

            HStack(spacing: 16) {
                LoadingHeaderButton(title: String(localized: "Send"), systemImage: "arrow.up.right")
                LoadingHeaderButton(
                    title: String(localized: "Receive"),
                    systemImage: "arrow.down.left"
                )
            }
        }
        .padding()
        .padding(.vertical, 22)
        .padding(.top, safeAreaInsets.top + 75)
        .background(
            Image(.headerPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: 300, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .brightness(0.1)
        )
        .background(.midnightBlue)
    }
}

private struct LoadingHeaderButton: View {
    let title: String
    let systemImage: String

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: systemImage)
            Text(title)
        }
        .foregroundColor(Color.midnightBtn.opacity(0.6))
        .frame(maxWidth: .infinity)
        .padding()
        .padding(.vertical, 4)
        .background(Color.gray)
        .cornerRadius(10)
    }
}

private struct TransactionsLoadingCardView: View {
    var body: some View {
        VStack {
            VStack {
                HStack {
                    Text(String(localized: "Transactions"))
                        .foregroundStyle(.secondary)
                        .font(.subheadline)
                        .fontWeight(.bold)

                    Spacer()
                }
                .padding(.bottom, 12)

                EmptyWalletScanSpinnerState(message: TransactionsCopy.checkingWalletHistory)
                    .frame(maxWidth: .infinity)
                    .padding(.top, 56)

                Spacer()
                    .frame(minHeight: UIScreen.main.bounds.height * 0.2)
            }
            .padding()
            .padding(.top, 5)
        }
    }
}

#Preview {
    SelectedWalletContainer(id: WalletId())
        .environment(AppManager.shared)
}
