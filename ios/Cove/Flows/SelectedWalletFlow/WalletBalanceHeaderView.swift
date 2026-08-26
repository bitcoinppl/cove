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
    let balancePresentation: BalancePresentation
    @State var fiatBalance: Double? = nil
    @State var fiatPendingBalance: Double? = nil
    let metadata: WalletMetadata
    let updater: (WalletManagerAction) -> Void
    let showReceiveSheet: () -> Void

    private var initialScanIsIncomplete: Bool {
        manager.ledgerState.initialScanIncomplete
    }

    private var sendButtonIsUnavailable: Bool {
        metadata.walletType == .watchOnly || initialScanIsIncomplete
    }

    private var sendButtonForegroundColor: Color {
        sendButtonIsUnavailable ? Color.secondary : Color.midnightBtn
    }

    private var sendButtonBackgroundColor: Color {
        sendButtonIsUnavailable ? Color.gray : Color.btnPrimary
    }

    private func sendButtonPressed() {
        if metadata.walletType == .watchOnly {
            app.alertState = .init(.cantSendOnWatchOnlyWallet)
            return
        }

        if initialScanIsIncomplete {
            app.showInitialScanIncompleteAlert()
            return
        }

        if balance.asSats() == 0 {
            manager.errorAlert = TaggedItem(.noBalance)
            return
        }

        app.pushRoute(RouteFactory().sendSetAmount(id: metadata.id))
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
            WalletBalanceDisplay(
                balance: balance,
                fiatBalance: fiatBalance,
                fiatPendingBalance: fiatPendingBalance,
                balancePresentation: balancePresentation,
                metadata: metadata,
                manager: manager,
                fontSize: fontSize,
                eyeIcon: eyeIcon,
                toggleSensitiveVisibility: toggleSensitiveVisibility
            )

            WalletBalanceActions(
                sendForegroundColor: sendButtonForegroundColor,
                sendBackgroundColor: sendButtonBackgroundColor,
                send: sendButtonPressed,
                receive: showReceiveSheet
            )
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
        .onAppear {
            if fiatBalance == nil {
                fiatBalance = manager.amountInFiatCached(balance)
            }

            if fiatPendingBalance == nil {
                fiatPendingBalance = manager.amountInFiatCached(manager.balance.untrustedPending())
            }
        }
        .onChange(of: manager.balance, initial: false) { _, newBalance in
            // recalculate fiat when balance changes
            fiatBalance = manager.amountInFiatCached(newBalance.spendable())
            fiatPendingBalance = manager.amountInFiatCached(newBalance.untrustedPending())
        }
        .onChange(of: app.prices, initial: false) { _, _ in
            // recalculate fiat when prices are loaded/updated
            fiatBalance = manager.amountInFiatCached(balance)
            fiatPendingBalance = manager.amountInFiatCached(manager.balance.untrustedPending())
        }
    }

    private func toggleSensitiveVisibility() {
        updater(.toggleSensitiveVisibility)
    }
}

private struct WalletBalanceDisplay: View {
    let balance: Amount
    let fiatBalance: Double?
    let fiatPendingBalance: Double?
    let balancePresentation: BalancePresentation
    let metadata: WalletMetadata
    let manager: WalletManager
    let fontSize: CGFloat
    let eyeIcon: String
    let toggleSensitiveVisibility: () -> Void

    var body: some View {
        VStack(spacing: 6) {
            HStack {
                WalletBalanceSecondaryView(
                    balance: balance,
                    fiatBalance: fiatBalance,
                    metadata: metadata,
                    manager: manager,
                    opacity: balancePresentation.secondaryOpacity
                )
                .foregroundColor(.white.opacity(balancePresentation.secondaryOpacity))
                .font(.footnote)
                .padding(.leading, 2)
                .contentTransition(.numericText())

                Spacer()
            }

            HStack {
                WalletBalancePrimaryView(
                    balance: balance,
                    fiatBalance: fiatBalance,
                    metadata: metadata,
                    manager: manager,
                    opacity: balancePresentation.primaryOpacity
                )
                .foregroundStyle(.white.opacity(balancePresentation.primaryOpacity))
                .font(.system(size: fontSize, weight: .bold))
                .contentTransition(.numericText())

                Spacer()

                Image(systemName: eyeIcon)
                    .foregroundColor(.gray)
                    .onTapGesture(perform: toggleSensitiveVisibility)
            }

            WalletBalancePendingView(
                pending: manager.balance.untrustedPending(),
                fiatPendingBalance: fiatPendingBalance,
                metadata: metadata,
                manager: manager,
                opacity: balancePresentation.pendingOpacity
            )
        }
        .onTapGesture(perform: togglePrimarySecondary)
        .contentShape(
            .contextMenuPreview,
            RoundedRectangle(cornerRadius: 8).inset(by: -8)
        )
        .contextMenu {
            Button("BTC", action: useBtc)
            Button("SATS", action: useSats)
        }
    }

    private func togglePrimarySecondary() {
        manager.dispatch(action: .toggleFiatBtcPrimarySecondary)
    }

    private func useBtc() {
        manager.dispatch(action: .updateUnit(.btc))
        manager.dispatch(action: .updateFiatOrBtc(.btc))
    }

    private func useSats() {
        manager.dispatch(action: .updateUnit(.sat))
        manager.dispatch(action: .updateFiatOrBtc(.btc))
    }
}

private struct WalletBalanceActions: View {
    let sendForegroundColor: Color
    let sendBackgroundColor: Color
    let send: () -> Void
    let receive: () -> Void

    var body: some View {
        HStack(spacing: 16) {
            Button(action: send) {
                HStack(spacing: 10) {
                    Image(systemName: "arrow.up.right")
                    Text("Send")
                }
                .foregroundColor(sendForegroundColor)
                .frame(maxWidth: .infinity)
                .padding()
                .padding(.vertical, 4)
                .background(sendBackgroundColor)
                .cornerRadius(10)
            }

            Button(action: receive) {
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
}

private struct WalletBalancePrimaryView: View {
    let balance: Amount
    let fiatBalance: Double?
    let metadata: WalletMetadata
    let manager: WalletManager
    let opacity: Double

    var body: some View {
        if !metadata.sensitiveVisible {
            Text("••••••")
        } else if metadata.fiatOrBtc == .fiat {
            if let fiatBalance {
                Text(manager.displayFiatAmount(fiatBalance))
            } else {
                ProgressView()
                    .tint(.white.opacity(opacity))
            }
        } else {
            Text(manager.amountFmtUnit(balance))
        }
    }
}

private struct WalletBalanceSecondaryView: View {
    let balance: Amount
    let fiatBalance: Double?
    let metadata: WalletMetadata
    let manager: WalletManager
    let opacity: Double

    var body: some View {
        if !metadata.sensitiveVisible {
            Text("••••••")
        } else if metadata.fiatOrBtc == .btc {
            if let fiatBalance {
                Text(manager.displayFiatAmount(fiatBalance))
            } else {
                ProgressView()
                    .tint(.white.opacity(opacity))
                    .scaleEffect(0.7)
            }
        } else {
            Text(manager.amountFmtUnit(balance))
        }
    }
}

private struct WalletBalancePendingView: View {
    let pending: Amount
    let fiatPendingBalance: Double?
    let metadata: WalletMetadata
    let manager: WalletManager
    let opacity: Double

    var body: some View {
        if metadata.fiatOrBtc == .fiat, let fiatPendingBalance,
           let pendingStr = manager.displayFiatAmountPendingFmt(fiatPendingBalance)
        {
            pendingText(pendingStr)
        } else if let pendingStr = manager.displayAmountPendingFmt(pending) {
            pendingText(pendingStr)
        }
    }

    private func pendingText(_ pendingStr: String) -> some View {
        HStack {
            Text(pendingStr)
                .foregroundColor(.white.opacity(opacity))
                .font(.footnote)
                .padding(.leading, 2)
            Spacer()
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
                balancePresentation: RustWalletManager.previewNewWallet()
                    .balancePresentation(scanStatus: .idle),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: .only))
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
                balancePresentation: RustWalletManager.previewNewWallet()
                    .balancePresentation(scanStatus: .idle),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: .only))
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
                balancePresentation: RustWalletManager.previewNewWallet()
                    .balancePresentation(scanStatus: .idle),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: .only))
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
                balancePresentation: RustWalletManager.previewNewWallet()
                    .balancePresentation(scanStatus: .idle),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: .only))
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
                balancePresentation: RustWalletManager.previewNewWallet()
                    .balancePresentation(scanStatus: .idle),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: .only))
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
                balancePresentation: RustWalletManager.previewNewWallet()
                    .balancePresentation(scanStatus: .idle),
                fiatBalance: 1835.00,
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .environment(AppManager.shared)
            .environment(WalletManager(preview: .only))
        }
}
