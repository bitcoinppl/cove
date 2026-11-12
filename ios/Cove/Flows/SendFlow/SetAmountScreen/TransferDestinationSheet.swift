//
//  TransferDestinationSheet.swift
//  Cove
//

import SwiftUI

/// Lets the user send to another of their wallets instead of typing or scanning an address
struct TransferDestinationSheet: View {
    @Environment(\.dismiss) private var dismiss

    let destinations: [TransferDestination]
    let onSelect: (WalletId) -> Void

    private var hasHardwareWallet: Bool {
        destinations.contains { $0.metadata.walletType == .cold }
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ForEach(destinations, id: \.metadata.id) { destination in
                        TransferDestinationRow(destination: destination) {
                            dismiss()
                            onSelect(destination.metadata.id)
                        }
                    }
                } header: {
                    Text("Cove fills in that wallet's next receive address.")
                        .textCase(nil)
                } footer: {
                    // cove derives a hardware wallet's address from its public key, the device
                    // never saw it
                    if hasHardwareWallet {
                        Text(
                            "Cove derives a hardware wallet's address from its public key. Check the address on the device before sending a large amount."
                        )
                    }
                }
            }
            .navigationTitle("Send to My Wallet")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

private struct TransferDestinationRow: View {
    let destination: TransferDestination
    let select: () -> Void

    private var metadata: WalletMetadata { destination.metadata }
    private var isReady: Bool { destination.status == .ready }

    private var detail: String? {
        if !isReady { return "Open this wallet once so Cove can scan it" }

        switch metadata.walletType {
        case .cold: return "Hardware wallet"
        case .xpubOnly, .watchOnly: return "Watch-only"
        case .hot: return nil
        }
    }

    var body: some View {
        Button(action: select) {
            HStack(spacing: 12) {
                Circle()
                    .fill(Color(metadata.color))
                    .frame(width: 10, height: 10)

                VStack(alignment: .leading, spacing: 2) {
                    Text(metadata.name)
                        .foregroundStyle(.primary)
                        .lineLimit(1)

                    if let detail {
                        Text(detail)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }

                Spacer()
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!isReady)
        .opacity(isReady ? 1 : 0.5)
        .accessibilityIdentifier("sendFlow.transferDestination.\(metadata.id)")
    }
}
