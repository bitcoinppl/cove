//
//  TransactionsCardView.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import SwiftUI

struct TransactionsCardView: View {
    let transactions: [Transaction]
    let scanComplete: Bool
    let metadata: WalletMetadata

    private let screenHeight = UIScreen.main.bounds.height

    @ViewBuilder
    func TransactionRow(_ txn: Transaction) -> some View {
        VStack(alignment: .leading) {
            Group {
                switch txn {
                case let .confirmed(txn):
                    ConfirmedTransactionView(txn: txn, metadata: metadata)
                case let .unconfirmed(txn):
                    UnconfirmedTransactionView(transaction: txn)
                }
            }
            .padding(.vertical, 6)

            Divider().opacity(0.7)
        }
    }

    var body: some View {
        VStack {
            VStack {
                HStack {
                    Text("Transactions")
                        .foregroundStyle(.secondary)
                        .font(.subheadline)
                        .fontWeight(.bold)
                    Spacer()
                }
                .padding(.bottom, 12)

                LazyVStack(alignment: .leading) {
                    ForEach(transactions, content: TransactionRow)
                }

                if transactions.isEmpty {
                    VStack {
                        ContentUnavailableView {
                            Label("No transactions", systemImage: "bitcoinsign.square.fill")
                        } description: {
                            Text("Send some bitcoin to yourself")
                        }
                        .padding(.top, 20)

                        Spacer()
                            .frame(minHeight: screenHeight * 0.2)
                    }
                    .background(.thickMaterial)
                }
            }
            .padding()
            .padding(.top, 5)
        }
    }
}

struct ConfirmedTransactionView: View {
    let txn: ConfirmedTransaction
    let metadata: WalletMetadata

    func amount(_ sentAndReceived: SentAndReceived) -> String {
        if !metadata.sensitiveVisible {
            return "**************"
        }

        return sentAndReceived.amountFmt(unit: metadata.selectedUnit)
    }

    func amountColor(_ direction: TransactionDirection) -> Color {
        switch direction {
        case .incoming:
            .green
        case .outgoing:
            .primary.opacity(0.8)
        }
    }

    var body: some View {
        HStack {
            TxnIcon(direction: txn.sentAndReceived().direction())
            VStack(alignment: .leading, spacing: 5) {
                Text(txn.label())
                    .font(.subheadline)
                    .fontWeight(.medium)
                    .foregroundColor(.primary.opacity(0.65))

                Text(txn.confirmedAtFmt())
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing) {
                Text(amount(txn.sentAndReceived()))
                    .foregroundStyle(amountColor(txn.sentAndReceived().direction()))
                Text(txn.blockHeightFmt())
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
    }
}

struct UnconfirmedTransactionView: View {
    let transaction: UnconfirmedTransaction

    var body: some View {
        HStack {
//            Text("Unconfirmed")
        }
    }
}

private struct TxnIcon: View {
    @Environment(\.colorScheme) var colorScheme

    let direction: TransactionDirection

    var iconColor: Color {
        colorScheme == .dark ? .gray.opacity(0.35) : .primary.opacity(0.75)
    }

    var arrow: String {
        switch direction {
        case .incoming:
            "arrow.down.left"
        case .outgoing:
            "arrow.down.right"
        }
    }

    var body: some View {
        Image(systemName: arrow)
            .foregroundColor(.white)
            .padding()
            .background(iconColor)
            .cornerRadius(6)
            .padding(.trailing, 5)
    }
}

#Preview("Full of Txns - Complete") {
    TransactionsCardView(
        transactions: transactionsPreviewNew(confirmed: UInt8(25), unconfirmed: UInt8(0)),
        scanComplete: true,
        metadata: walletMetadataPreview()
    )
}

#Preview("Full of Txns - Scanning") {
    TransactionsCardView(
        transactions: transactionsPreviewNew(confirmed: UInt8(25), unconfirmed: UInt8(1)),
        scanComplete: false,
        metadata: walletMetadataPreview()
    )
}

#Preview("Empty - Scanning") {
    TransactionsCardView(transactions: [], scanComplete: false, metadata: walletMetadataPreview())
}

#Preview("Empty") {
    VStack {
        Text("Test")

        Spacer()
        ScrollView {
            TransactionsCardView(transactions: [], scanComplete: true, metadata: walletMetadataPreview())
                .background(
                    UnevenRoundedRectangle(
                        cornerRadii: .init(
                            topLeading: 40,
                            bottomLeading: 0,
                            bottomTrailing: 0,
                            topTrailing: 40
                        )
                    )
                    .fill(.thickMaterial)
                    .ignoresSafeArea()
                )
        }
        .ignoresSafeArea()
    }
}
