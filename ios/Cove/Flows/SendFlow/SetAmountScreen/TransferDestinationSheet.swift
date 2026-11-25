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

    var body: some View {
        VStack(spacing: 0) {
            TransferDestinationHeader(close: { dismiss() })

            TransferDestinationList(destinations: destinations) { id in
                dismiss()
                onSelect(id)
            }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }
}

private struct TransferDestinationHeader: View {
    let close: () -> Void

    var body: some View {
        HStack {
            Text("Your Wallets")
                .font(.title)
                .fontWeight(.bold)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier("sendFlow.transferDestinations.title")

            Spacer()

            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(.secondary)
                    .frame(width: 30, height: 30)
                    .background(Circle().fill(Color.systemGray6))
            }
            .accessibilityLabel("Close")
            .accessibilityIdentifier("sendFlow.transferDestinations.close")
        }
        .padding(.horizontal, 20)
        .padding(.top, 28)
        .padding(.bottom, 8)
    }
}

private struct TransferDestinationList: View {
    let destinations: [TransferDestination]
    let select: (WalletId) -> Void

    var body: some View {
        List(destinations, id: \.metadata.id) { destination in
            TransferDestinationRow(destination: destination) {
                select(destination.metadata.id)
            }
            .listRowInsets(EdgeInsets(top: 12, leading: 20, bottom: 12, trailing: 20))
        }
        .listStyle(.plain)
    }
}

private struct TransferDestinationRow: View {
    let destination: TransferDestination
    let select: () -> Void

    private var metadata: WalletMetadata { destination.metadata }
    private var isReady: Bool { destination.status == .ready }

    /// A wallet that hasn't finished its first scan can't hand out an address yet, so its row
    /// says how to fix that instead of naming its keys
    private var detail: String {
        guard isReady else { return "Open this wallet once so Cove can scan it" }
        return metadata.identOrFingerprint()
    }

    var body: some View {
        Button(action: select) {
            HStack(spacing: 14) {
                Circle()
                    .fill(Color(metadata.color))
                    .frame(width: 14, height: 14)

                VStack(alignment: .leading, spacing: 4) {
                    Text(metadata.name)
                        .font(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(.primary)
                        .lineLimit(1)

                    Text(detail)
                        .font(.footnote)
                        .fontDesign(isReady ? .monospaced : .default)
                        .foregroundStyle(.secondary)
                }

                Spacer()
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!isReady)
        .opacity(isReady ? 1 : 0.5)
        .accessibilityLabel(metadata.name)
        .accessibilityValue(detail)
        .accessibilityIdentifier("sendFlow.transferDestination.\(metadata.id)")
    }
}
