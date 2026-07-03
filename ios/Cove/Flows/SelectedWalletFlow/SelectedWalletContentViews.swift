//
//  SelectedWalletContentViews.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/26.
//

import CoveCore
import SwiftUI

struct SelectedWalletLoadingView: View {
    let screenHeight: CGFloat

    var body: some View {
        Spacer()

        ProgressView()
            .padding(.top, screenHeight / 6)
            .tint(.primary)

        Spacer()
        Spacer()
    }
}

struct SelectedWalletTransactionsView: View {
    let loadState: WalletLoadState
    let unsignedTransactions: [UnsignedTransaction]
    let metadata: WalletMetadata
    let screenHeight: CGFloat

    var body: some View {
        switch loadState {
        case .loading:
            SelectedWalletLoadingView(screenHeight: screenHeight)
        case let .scanning(transactions):
            transactionsCard(transactions: transactions)
        case let .loaded(transactions):
            transactionsCard(transactions: transactions)
        }
    }

    private func transactionsCard(transactions: [CoveCore.Transaction]) -> some View {
        TransactionsCardView(
            transactions: transactions,
            unsignedTransactions: unsignedTransactions,
            metadata: metadata
        )
        .ignoresSafeArea()
        .background(Color.coveBg)
    }
}

struct SelectedWalletTitleContent: View {
    let metadata: WalletMetadata
    let toolbarTextColor: Color
    let changeName: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            if metadata.walletType == .cold || metadata.walletType == .xpubOnly {
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
        .contentShape(Rectangle())
        .contentShape(
            .contextMenuPreview,
            RoundedRectangle(cornerRadius: 8)
        )
        .contextMenu {
            Button("Change Name", action: changeName)
        }
    }
}

struct SelectedWalletMainContent: View {
    let manager: WalletManager
    let screenHeight: CGFloat
    let cloudBackupIsConfigured: Bool
    let updater: (WalletManagerAction) -> Void
    let showReceiveSheet: () -> Void
    let headerBottomChanged: (CGFloat) -> Void

    var body: some View {
        VStack(spacing: 0) {
            WalletBalanceHeaderView(
                balance: manager.balance.spendable(),
                balancePresentation: manager.balancePresentation,
                metadata: manager.walletMetadata,
                updater: updater,
                showReceiveSheet: showReceiveSheet
            )
            .clipped()
            .onGeometryChange(for: CGFloat.self) { proxy in
                proxy.frame(in: .global).maxY
            } action: { _, headerBottom in
                headerBottomChanged(headerBottom)
            }

            if !cloudBackupIsConfigured {
                VerifyReminder(
                    walletId: manager.walletMetadata.id,
                    isVerified: manager.walletMetadata.verified
                )
            }

            SelectedWalletTransactionsView(
                loadState: manager.loadState,
                unsignedTransactions: manager.unsignedTransactions,
                metadata: manager.walletMetadata,
                screenHeight: screenHeight
            )
            .environment(manager)
        }
        .background(Color.coveBg)
    }
}
