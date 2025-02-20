//
//  SendFlowDetailsSheetView.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import SwiftUI

struct SendFlowDetailsSheetView: View {
    @Environment(\.dismiss) private var dismiss

    let manager: WalletManager
    let details: ConfirmDetails

    var body: some View {
        VStack(spacing: 24) {
            Text("More Details")
                .font(.callout)
                .fontWeight(.semibold)
                .padding(.top)

            SendFlowAccountSection(manager: manager)

            Divider()

            SendFlowDetailsView(
                manager: manager,
                details: details,
                prices: nil
            )

            Spacer()

            Button(action: { dismiss() }) {
                Text("Close")
                    .padding(.vertical, 12)
                    .frame(maxWidth: .infinity)
            }
            .font(.caption)
            .background(.midnightBtn)
            .foregroundStyle(.white)
            .cornerRadius(10)
        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowDetailsSheetView(
            manager: WalletManager(preview: "preview_only"),
            details: ConfirmDetails.previewNew()
        )
        .padding()
    }
}
