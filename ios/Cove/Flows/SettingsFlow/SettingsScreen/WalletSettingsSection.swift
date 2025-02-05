//
//  WalletSettingsSection.swift
//  Cove
//
//  Created by Praveen Perera on 2/3/25.
//

import SwiftUI

struct WalletSettingsSection: View {
    @State var wallets: [WalletMetadata]

    init(wallets: [WalletMetadata]? = nil) {
        if let wallets { self.wallets = wallets; return }

        do {
            self.wallets = try Database().wallets().allSortedActive()
            Log.debug("Wallets: \(self.wallets)")
        } catch {
            Log.error("Failed to get wallets \(error)")
            self.wallets = []
        }
    }

    func WalletIcon(_ wallet: WalletMetadata) -> SettingsIcon {
        let foregroundColor = switch wallet.swiftColor {
        case .almostWhite: Color.black
        case .lightMint: Color.black
        default: Color.white
        }

        return SettingsIcon(symbol: "wallet.bifold", foregroundColor: foregroundColor, backgroundColor: wallet.swiftColor)
    }

    private var topAmount = 5
    private var top5Wallets: [WalletMetadata] {
        wallets.count > topAmount ? Array(wallets[0 ... topAmount - 1]) : wallets
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
        .onAppear {
            if let wallets = try? Database().wallets().allSortedActive() {
                self.wallets = wallets
            }
        }
    }
}

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
