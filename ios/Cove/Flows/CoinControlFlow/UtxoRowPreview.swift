//
//  UtxoRowPreview.swift
//  Cove
//
//  Created by Praveen Perera on 5/26/25.
//

import SwiftUI

struct UtxoRowPreview: View {
    let displayAmount: (Amount) -> String
    let utxo: Utxo

    var body: some View {
        VStack {
            Text(utxo.address.spacedOut())
                .font(.footnote)
                .fontWeight(.semibold)
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)

            Spacer()

            HStack(alignment: .center, spacing: 8) {
                Text(utxo.name)
                    .foregroundColor(.primary)
                    .font(.body)
                    .fontWeight(.medium)

                if utxo.type == .change {
                    Image(systemName: "circlebadge.2")
                        .font(.caption)
                        .foregroundColor(.orange.opacity(0.8))
                }
            }

            Spacer()

            HStack {
                Text(displayAmount(utxo.amount))
                Spacer()
                Text(utxo.date)
            }
            .foregroundColor(.secondary)
            .font(.footnote)
        }
        .padding()
        .frame(idealWidth: screenWidth)
        .frame(maxWidth: .infinity)
        .frame(minHeight: 185)
    }
}

#Preview("UTXORowPreview") {
    AsyncPreview {
        let manager = CoinControlManager(RustCoinControlManager.previewNew())
        UtxoRowPreview(displayAmount: manager.displayAmount, utxo: manager.utxos[0])
            .environment(WalletManager(preview: "preview_only"))
    }
}
