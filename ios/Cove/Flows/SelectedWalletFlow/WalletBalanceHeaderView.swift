//
//  WalletBalanceHeaderView.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import SwiftUI

struct WalletBalanceHeaderView: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    @Environment(AppManager.self) var app
    @Environment(WalletManager.self) var manager

    // args
    // trusted spendable balance
    let balance: Amount
    @State var fiatBalance: Double? = nil
    let metadata: WalletMetadata
    let updater: (WalletManagerAction) -> Void
    let showReceiveSheet: () -> Void

    private var accentColor: Color {
        metadata.swiftColor
    }

    private var primaryBalanceString: String {
        if !metadata.sensitiveVisible {
            return "••••••"
        }

        // fiat
        if metadata.fiatOrBtc == .fiat {
            guard let fiatBalance else { return "$XX.XX USD" }
            return manager.rust.displayFiatAmount(amount: fiatBalance)
        }

        // btc or sats
        return manager.amountFmtUnit(balance)
    }

    private var secondaryBalanceString: String {
        if !metadata.sensitiveVisible {
            return "••••••"
        }

        // fiat
        if metadata.fiatOrBtc == .btc {
            guard let fiatBalance else { return "$XX.XX USD" }
            return manager.rust.displayFiatAmount(amount: fiatBalance)
        }

        // btc or sats
        return manager.amountFmtUnit(balance)
    }

    var eyeIcon: String {
        metadata.sensitiveVisible ? "eye" : "eye.slash"
    }

    var fontSize: CGFloat {
        let btc = balance.asBtc()

        // Base font size
        let baseFontSize: CGFloat = 34

        // Calculate the number of digits
        let digits = btc > 0 ? Int(log10(btc)) + 1 : 1

        // Reduce font size by 2 for each additional digit beyond 1
        let fontSizeReduction = CGFloat(max(0, (digits - 1) * 2))

        // Ensure minimum font size of 20
        return max(baseFontSize - fontSizeReduction, 20)
    }

    var body: some View {
        VStack(spacing: 28) {
            VStack(spacing: 6) {
                HStack {
                    Text(secondaryBalanceString)
                        .foregroundColor(.white.opacity(0.75))
                        .font(.footnote)
                        .padding(.leading, 2)
                        .contentTransition(.numericText())

                    Spacer()
                }

                HStack {
                    Text(primaryBalanceString)
                        .foregroundStyle(.white)
                        .font(.system(size: fontSize, weight: .bold))
                        .contentTransition(.numericText())

                    Spacer()

                    Image(systemName: eyeIcon)
                        .foregroundColor(.gray)
                        .onTapGesture { updater(.toggleSensitiveVisibility) }
                }
            }
            .onTapGesture {
                manager.dispatch(action: .toggleFiatBtcPrimarySecondary)
            }
            .contentShape(
                .contextMenuPreview,
                RoundedRectangle(cornerRadius: 8).inset(by: -8)
            )
            .contextMenu {
                Button("BTC") {
                    manager.dispatch(action: .updateUnit(.btc))
                    manager.dispatch(action: .updateFiatOrBtc(.btc))
                }

                Button("SATS") {
                    manager.dispatch(action: .updateUnit(.sat))
                    manager.dispatch(action: .updateFiatOrBtc(.btc))
                }
            }

            HStack(spacing: 16) {
                Button(action: {
                    if balance.asSats() == 0 {
                        manager.errorAlert = .noBalance
                        return
                    }

                    if metadata.walletType == .watchOnly {
                        app.alertState = .init(.cantSendOnWatchOnlyWallet)
                        return
                    }

                    app.pushRoute(RouteFactory().sendSetAmount(id: metadata.id))
                }) {
                    HStack(spacing: 10) {
                        Image(systemName: "arrow.up.right")
                        Text("Send")
                    }
                    .foregroundColor(
                        metadata.walletType == .watchOnly ? Color.secondary : Color.midnightBtn
                    )
                    .frame(maxWidth: .infinity)
                    .padding()
                    .padding(.vertical, 4)
                    .background(
                        metadata.walletType == .watchOnly ? Color.gray : Color.btnPrimary
                    )
                    .cornerRadius(10)
                }

                Button(action: showReceiveSheet) {
                    HStack(spacing: 10) {
                        Image(systemName: "arrow.down.left")
                        Text("Receive")
                    }
                    .foregroundColor(Color.midnightBtn)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .padding(.vertical, 4)
                    .background(Color.btnPrimary)
                    .cornerRadius(10)
                }
            }
        }
        .padding()
        .padding(.vertical, 22)
        .padding(.top, safeAreaInsets.top + 35)
        .background(
            Image(.headerPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: 300, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .brightness(0.1)
        )
        .background(.midnightBlue)
        .onChange(of: manager.fiatBalance, initial: false) {
            // if fiatBalance was pased in explicitly, don't update it, only for previews
            if fiatBalance ?? 0.0 > 0.0, manager.fiatBalance ?? 0.0 == 0.0 { return }
            fiatBalance = manager.fiatBalance
        }
        .onAppear {
            if fiatBalance != nil { return }
            fiatBalance = manager.fiatBalance
        }
        .task {
            if balance.asSats() != 0, fiatBalance == 0.00 || fiatBalance == nil {
                await manager.updateWalletBalance()
            }
        }
    }
}

#Preview("btc") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance: Amount.fromSat(sats: 1_000_738),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: "preview_only"))
        }
}

#Preview("sats") {
    var metadata = walletMetadataPreview()
    metadata.selectedUnit = .sat
    metadata.sensitiveVisible = true
    metadata.color = .blue

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance: Amount.fromSat(sats: 1_000_738),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: "preview_only"))
        }
}

#Preview("hidden") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = false
    metadata.color = .green

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance: Amount.fromSat(sats: 1_000_738),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: "preview_only"))
        }
}

#Preview("lots of btc") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true
    metadata.color = .purple

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance: Amount.fromSat(sats: 10_000_000_738),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: "preview_only"))
        }
}

#Preview("in fiat") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true
    metadata.color = .purple
    metadata.fiatOrBtc = .fiat

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance: Amount.fromSat(sats: 10_000_000_738),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: "preview_only"))
        }
}

#Preview("watch only") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true
    metadata.walletType = .watchOnly

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance: Amount.fromSat(sats: 10_000_000_738),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: "preview_only"))
        }
}
