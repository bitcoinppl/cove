//
//  SendFlowAccountSection.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import SwiftUI

struct SendFlowAccountSection: View {
    let model: WalletViewModel

    var metadata: WalletMetadata {
        model.walletMetadata
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

                if metadata.walletType == .cold {
                    BitcoinShieldIcon(width: 24, color: .orange)
                }

                VStack(alignment: .leading, spacing: 6) {
                    Text(
                        metadata.masterFingerprint?.asUppercase()
                            ?? "No Fingerprint"
                    )
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
            model: WalletViewModel(preview: "preview_only")
        )
    }
}
