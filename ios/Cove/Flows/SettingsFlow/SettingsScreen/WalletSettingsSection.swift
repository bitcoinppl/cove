//
//  WalletSettingsSection.swift
//  Cove
//
//  Created by Praveen Perera on 2/3/25.
//

import SwiftUI

struct WalletSettingsSection: View {
    @Environment(AppManager.self) private var app

    private let overrideWallets: [WalletMetadata]?

    init(wallets: [WalletMetadata]? = nil) {
        overrideWallets = wallets
    }

    func WalletIcon(_ wallet: WalletMetadata) -> SettingsIcon {
        let foregroundColor = switch wallet.swiftColor {
        case .almostWhite: Color.black
        case .lightMint: Color.black
        default: Color.white
        }

        return SettingsIcon(symbol: "wallet.bifold", foregroundColor: foregroundColor, backgroundColor: wallet.swiftColor)
    }

    private let topAmount = 5
    private var wallets: [WalletMetadata] {
        overrideWallets ?? app.wallets
    }

    private var top5Wallets: [WalletMetadata] {
        Array(wallets.prefix(topAmount))
    }

    var body: some View {
        Section("Wallet Settings") {
            ForEach(top5Wallets) { wallet in
                SettingsRow(
                    title: wallet.name,
                    route: .wallet(id: wallet.id, route: .main),
                    icon: WalletIcon(wallet)
                )
            }

            if wallets.count > topAmount {
                SettingsRow(
                    title: "More",
                    route: .allWallets,
                    icon: SettingsIcon(
                        symbol: "ellipsis",
                        foregroundColor: .secondary,
                        backgroundColor: .clear
                    )
                )
            }
        }
    }
}

#if DEBUG
    #Preview {
        Form {
            WalletSettingsSection(wallets: [
                WalletMetadata("Test 1", preview: true),
                WalletMetadata("Test 2", preview: true),
                WalletMetadata("Test 3", preview: true),
                WalletMetadata("Test 4", preview: true),
                WalletMetadata("Test 5", preview: true),
                WalletMetadata("Test 6", preview: true),
            ])
        }
        .environment(AppManager.shared)
    }
#endif
