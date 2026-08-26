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

    public var body: some View {
        VStack(spacing: 32) {
            Text("Multiple wallets found, please choose one")
            Text("Multiple wallets found, please choose one")
                .font(.title)
                .fontWeight(.bold)
                .multilineTextAlignment(.center)

            CurrentWalletTypeButton(
                address: address?.addressUnformatted() ?? "bc1q",
                select: selectCurrentWallet
            )

            ForEach(foundAddressesSorted, id: \.self) { foundAddress in
                FoundWalletTypeButton(
                    manager: manager,
                    foundAddress: foundAddress,
                    dismiss: dismiss.callAsFunction
                )
            }
        }
        .task {
            await loadAddress()
        }
        .padding()
    }

    private func selectCurrentWallet() {
        manager.dispatch(action: .selectCurrentWalletAddressType)
        dismiss()
    }

    private func loadAddress() async {
        let address = try? await manager.firstAddress()
        guard let address else { return }

        withAnimation {
            self.address = address
        }
    }
}

private struct CurrentWalletTypeButton: View {
    let address: String
    let select: () -> Void

    var body: some View {
        Button(action: select) {
            VStack {
                Text("Keep Current")
                    .font(.title3)
                    .fontWeight(.semibold)
                    .foregroundStyle(.blue)

                Text(address)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct FoundWalletTypeButton: View {
    let manager: WalletManager
    let foundAddress: FoundAddress
    let dismiss: () -> Void

    var body: some View {
        Button(action: select) {
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

    private func select() {
        Task {
            do {
                _ = try await manager.switchToDifferentWalletAddressType(foundAddress.type)
            } catch {
                Log.error(error.localizedDescription)
                dismiss()
                return
            }

            await MainActor.run {
                dismiss()
            }
        }
    }
}

#Preview {
    AsyncPreview {
        ChooseWalletTypeView(
            manager: WalletManager(preview: .only),
            foundAddresses: [
                previewNewLegacyFoundAddress(),
                previewNewWrappedFoundAddress(),
            ]
        )
    }
}
