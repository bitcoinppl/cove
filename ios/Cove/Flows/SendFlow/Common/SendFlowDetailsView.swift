//
//  SendFlowDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import SwiftUI

struct SendFlowDetailsView: View {
    let manager: WalletManager
    let details: ConfirmDetails

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var body: some View {
        VStack(spacing: 12) {
            // To Address Section
            HStack {
                Text("Address")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundStyle(.secondary)
                    .foregroundColor(.primary)

                Spacer()
                Spacer()
                Spacer()
                Spacer()

                Text(
                    details
                        .sendingTo()
                        .spacedOut()
                )
                .font(.system(.footnote, design: .none))
                .fontWeight(.semibold)
                .padding(.leading, 60)
                .lineLimit(3)
            }
            .padding(.top, 6)

            // Network Fee Section
            HStack {
                Text("Network Fee")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundStyle(.secondary)

                Spacer()

                HStack {
                    Text(manager.amountFmt(details.feeTotal()))
                    Text(metadata.selectedUnit == .sat ? "sats" : "btc")
                }
                .font(.footnote)
                .fontWeight(.medium)
                .foregroundStyle(.secondary)
                .padding(.vertical, 10)
            }

            // They receive section
            HStack {
                Text("They'll receive")
                Spacer()
                HStack {
                    Text(manager.amountFmt(details.sendingAmount()))
                    Text(metadata.selectedUnit == .sat ? "sats" : "btc")
                }
            }
            .font(.footnote)
            .fontWeight(.semibold)

            // Total Amount Section
            HStack {
                Text("You'll pay")
                Spacer()
                HStack {
                    Text(manager.amountFmt(details.spendingAmount()))
                    Text(metadata.selectedUnit == .sat ? "sats" : "btc")
                }
            }
            .font(.footnote)
            .fontWeight(.semibold)

        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowDetailsView(
            manager: WalletManager(preview: "preview_only"),
            details: ConfirmDetails.previewNew()
        )
        .padding()
    }
}
