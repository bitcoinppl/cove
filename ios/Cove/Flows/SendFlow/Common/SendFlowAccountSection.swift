//
//  SendFlowAccountSection.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import SwiftUI

struct SendFlowAccountSection: View {
    let manager: WalletManager

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var body: some View {
        VStack(spacing: 16) {
            HStack {
                if metadata.walletType == .hot {
                    Image(systemName: "bitcoinsign")
                        .font(.title2)
                        .foregroundColor(.orange)
                        .padding(.trailing, 6)
                }

                if case .cold = metadata.walletType {
                    BitcoinShieldIcon(width: 24, color: .orange)
                }

                VStack(alignment: .leading, spacing: 6) {
                    Text(metadata.identOrFingerprint())
                        .font(.footnote)
                        .fontWeight(.medium)
                        .foregroundColor(.secondary)

                    Text(metadata.name)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }

                Spacer()
            }
        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowAccountSection(
            manager: WalletManager(preview: "preview_only")
        )
    }
}
