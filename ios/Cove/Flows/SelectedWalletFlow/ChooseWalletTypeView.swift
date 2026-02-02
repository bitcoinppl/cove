//
//  ChooseWalletTypeView.swift
//  Cove
//
//  Created by Praveen Perera on 10/20/24.
//

import Foundation
import SwiftUI

public struct ChooseWalletTypeView: View {
    @Environment(\.dismiss) private var dismiss
    @State var manager: WalletManager
    @State var foundAddresses: [FoundAddress]

    /// private
    /// first native segwit address
    @State private var address: AddressInfo? = nil

    var foundAddressesSorted: [FoundAddress] {
        foundAddresses.sorted { x1, x2 in x2.type > x1.type }
    }

    func TypeButton(_ foundAddress: FoundAddress) -> some View {
        Button(action: {
            Task {
                // switch the wallet
                do {
                    try await manager.rust.switchToDifferentWalletAddressType(
                        walletAddressType: foundAddress.type
                    )
                } catch {
                    Log.error(error.localizedDescription)
                    dismiss()
                    return
                }

                // update the metadata
                await MainActor.run {
                    manager.dispatch(action: .selectDifferentWalletAddressType(foundAddress.type))
                    dismiss()
                }
            }
        }) {
            VStack {
                Text(String(foundAddress.type))
                    .font(.title3)
                    .fontWeight(.semibold)

                Text(foundAddress.firstAddress)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(.primary)
    }

    public var body: some View {
        VStack(spacing: 32) {
            Text("Multiple wallets found, please choose one")
            Text("Multiple wallets found, please choose one")
                .font(.title)
                .fontWeight(.bold)
                .multilineTextAlignment(.center)

            Button(action: {
                manager.dispatch(action: .selectCurrentWalletAddressType)
                dismiss()
            }) {
                VStack {
                    Text("Keep Current")
                        .font(.title3)
                        .fontWeight(.semibold)
                        .foregroundStyle(.blue)

                    Text(address?.addressUnformatted() ?? "bc1q")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            }

            ForEach(foundAddressesSorted, id: \.self, content: TypeButton)
        }
        .task {
            let address = try? await manager.firstAddress()
            if let address {
                withAnimation {
                    self.address = address
                }
            }
        }
        .padding()
    }
}

#Preview {
    AsyncPreview {
        ChooseWalletTypeView(
            manager: WalletManager(preview: "preview_only"),
            foundAddresses: [
                previewNewLegacyFoundAddress(),
                previewNewWrappedFoundAddress(),
            ]
        )
    }
}
