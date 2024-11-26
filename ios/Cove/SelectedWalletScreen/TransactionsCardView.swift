//
//  TransactionsCardView.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import MijickPopupView
import SwiftUI

struct TransactionsCardView: View {
    let transactions: [Transaction]
    let unsignedTransactions: [UnsignedTransaction]
    let scanComplete: Bool
    let metadata: WalletMetadata

    private let screenHeight = UIScreen.main.bounds.height

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

                if !scanComplete {
                    ProgressView()
                        .tint(.primary)
                        .padding(.bottom, 10)
                }

                LazyVStack(alignment: .leading) {
                    ForEach(unsignedTransactions) { txn in
                        VStack(alignment: .leading) {
                            Group {
                                UnsignedTransactionView(txn: txn, metadata: metadata)
                            }
                            .padding(.vertical, 6)

                            Divider().opacity(0.7)
                        }
                    }

                    ForEach(transactions) { txn in
                        TransactionRow(txn: txn, metadata: metadata)
                    }
                }

                if transactions.isEmpty {
                    VStack {
                        ContentUnavailableView {
                            Label("No transactions", systemImage: "bitcoinsign.square.fill")
                        } description: {
                            Text("Go buy some bitcoin!")
                        }
                        .padding(.top, 20)

                        Spacer()
                            .frame(minHeight: screenHeight * 0.2)
                    }
                }
            }
            .padding()
            .padding(.top, 5)
        }
        .onDisappear {
            PopupManager.dismiss()
        }
    }
}

private func amountColor(_ direction: TransactionDirection) -> Color {
    switch direction {
    case .incoming:
        .green
    case .outgoing:
        .primary.opacity(0.8)
    }
}

struct TransactionRow: View {
    @Environment(WalletViewModel.self) var model
    var txn: Transaction
    var metadata: WalletMetadata

    var body: some View {
        VStack(alignment: .leading) {
            Group {
                switch txn {
                case let .confirmed(txn):
                    ConfirmedTransactionView(txn: txn, metadata: metadata)
                case let .unconfirmed(txn):
                    UnconfirmedTransactionView(txn: txn, metadata: metadata)
                }
            }
            .padding(.vertical, 6)

            Divider().opacity(0.7)
        }
    }
}

struct ConfirmedTransactionView: View {
    @Environment(\.navigate) private var navigate
    @Environment(WalletViewModel.self) var model

    let txn: ConfirmedTransaction
    let metadata: WalletMetadata

    // private
    @State private var transactionDetails: TransactionDetails? = nil
    @State private var loading: Bool = false

    private var amount: String {
        if case .btc = metadata.fiatOrBtc {
            return privateShow(
                model.rust.displaySentAndReceivedAmount(sentAndReceived: txn.sentAndReceived())
            )
        }

        // fiat
        guard let fiatAmount = txn.fiatAmount() else { return privateShow("---") }
        return privateShow(model.rust.displayFiatAmount(amount: fiatAmount.amount))
    }

    private func privateShow(_ text: String, placeholder: String = "*******") -> String {
        if !metadata.sensitiveVisible {
            placeholder
        } else {
            text
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

                Text(privateShow(txn.confirmedAtFmt()))
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing) {
                Text(amount)
                    .foregroundStyle(amountColor(txn.sentAndReceived().direction()))
                Text(privateShow(txn.blockHeightFmt()))
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .onTapGesture {
            MiddlePopup(state: .loading).showAndStack()
            Task {
                do {
                    let details = try await model.rust.transactionDetails(txId: txn.id())
                    await MainActor.run {
                        PopupManager.dismiss()
                        navigate(Route.transactionDetails(id: metadata.id, details: details))
                    }
                } catch {
                    Log.error(
                        "Unable to get transaction details: \(error.localizedDescription), for txn: \(txn.id())"
                    )
                }
            }
        }
    }
}

struct UnconfirmedTransactionView: View {
    @Environment(\.navigate) private var navigate
    @Environment(WalletViewModel.self) var model

    let txn: UnconfirmedTransaction
    let metadata: WalletMetadata

    func privateShow(_ text: String, placeholder: String = "*******") -> String {
        if !metadata.sensitiveVisible {
            placeholder
        } else {
            text
        }
    }

    private var amount: String {
        if case .btc = metadata.fiatOrBtc {
            return privateShow(
                model.rust.displaySentAndReceivedAmount(sentAndReceived: txn.sentAndReceived())
            )
        }

        // fiat
        if let fiatAmount = txn.fiatAmount() {
            return privateShow(model.rust.displayFiatAmount(amount: fiatAmount.amount))
        } else {
            return privateShow("---")
        }
    }

    var body: some View {
        HStack {
            TxnIcon(direction: txn.sentAndReceived().direction(), confirmed: false)
                .opacity(0.6)

            VStack(alignment: .leading, spacing: 5) {
                Text(txn.label())
                    .font(.subheadline)
                    .fontWeight(.medium)
                    .foregroundColor(.primary.opacity(0.4))
            }
            Spacer()
            VStack(alignment: .trailing) {
                Text(amount)
                    .foregroundStyle(amountColor(txn.sentAndReceived().direction()).opacity(0.65))
            }
        }.onTapGesture {
            MiddlePopup(state: .loading).showAndStack()
            Task {
                do {
                    let details = try await model.rust.transactionDetails(txId: txn.id())
                    await MainActor.run {
                        PopupManager.dismiss()
                        navigate(Route.transactionDetails(id: metadata.id, details: details))
                    }
                } catch {
                    Log.error(
                        "Unable to get transaction details: \(error.localizedDescription), for txn: \(txn.id())"
                    )
                }
            }
        }
    }
}

struct UnsignedTransactionView: View {
    @Environment(\.navigate) private var navigate
    @Environment(WalletViewModel.self) var model
    @Environment(\.colorScheme) var colorScheme

    // args
    let txn: UnsignedTransaction
    let metadata: WalletMetadata

    // private
    @State private var fiatAmount: Double? = nil

    func privateShow(_ text: String, placeholder: String = "*******") -> String {
        if !metadata.sensitiveVisible {
            placeholder
        } else {
            text
        }
    }

    private var amount: String {
        // btc or sats
        if case .btc = metadata.fiatOrBtc {
            return privateShow(model.amountFmtUnit(txn.spendingAmount()))
        }

        // fiat
        guard let fiatAmount else { return privateShow("---") }
        return privateShow(model.rust.displayFiatAmount(amount: fiatAmount))
    }

    var body: some View {
        HStack {
            Image(systemName: "lock.open.trianglebadge.exclamationmark")
                .symbolRenderingMode(.multicolor)
                .foregroundColor(.white)
                .padding()
                .frame(width: 50, height: 50)
                .background(colorScheme == .dark ? .gray.opacity(0.35) : .primary.opacity(0.75))
                .cornerRadius(6)
                .padding(.trailing, 5)
                .opacity(0.6)

            VStack(alignment: .leading, spacing: 5) {
                Text(txn.label())
                    .font(.subheadline)
                    .fontWeight(.medium)
                    .foregroundColor(.primary.opacity(0.4))

                Text("Pending Signature")
                    .font(.caption)
                    .fontWeight(.regular)
                    .foregroundStyle(.orange)
                    .opacity(0.8)
            }

            Spacer()

            VStack(alignment: .trailing) {
                Text(amount)
            }
        }
        .task {
            fiatAmount =
                try? await model.rust.amountInFiat(
                    amount: txn.spendingAmount(),
                    currency: .usd
                )
        }
        .onTapGesture {
            navigate(RouteFactory().sendConfirm(id: metadata.id, details: txn.details()))
        }
    }
}

private struct TxnIcon: View {
    @Environment(\.colorScheme) var colorScheme

    let direction: TransactionDirection
    var confirmed: Bool = true

    var iconColor: Color {
        colorScheme == .dark ? .gray.opacity(0.35) : .primary.opacity(0.75)
    }

    var arrow: String {
        if !confirmed {
            return "clock.arrow.2.circlepath"
        }

        switch direction {
        case .incoming:
            return "arrow.down.left"
        case .outgoing:
            return "arrow.up.right"
        }
    }

    var body: some View {
        Image(systemName: arrow)
            .foregroundColor(.white)
            .padding()
            .frame(width: 50, height: 50)
            .background(iconColor)
            .cornerRadius(6)
            .padding(.trailing, 5)
    }
}

#Preview("Full of Txns - Complete") {
    AsyncPreview {
        TransactionsCardView(
            transactions: transactionsPreviewNew(confirmed: UInt8(10), unconfirmed: UInt8(0)),
            unsignedTransactions: [],
            scanComplete: true,
            metadata: walletMetadataPreview()
        )
        .environment(WalletViewModel(preview: "preview_only"))
    }
}

#Preview("Full of Txns - Scanning") {
    AsyncPreview {
        ScrollView {
            TransactionsCardView(
                transactions: transactionsPreviewNew(confirmed: UInt8(10), unconfirmed: UInt8(1)),
                unsignedTransactions: [],
                scanComplete: false,
                metadata: walletMetadataPreview()
            )
            .background(.thickMaterial)
            .environment(WalletViewModel(preview: "preview_only"))
        }
    }
}

#Preview("Empty - Scanning") {
    AsyncPreview {
        TransactionsCardView(
            transactions: [],
            unsignedTransactions: [],
            scanComplete: false,
            metadata: walletMetadataPreview()
        )
        .environment(WalletViewModel(preview: "preview_only"))
    }
}

#Preview("With Unconfirmed Txns") {
    AsyncPreview {
        TransactionsCardView(
            transactions: transactionsPreviewNew(confirmed: UInt8(10), unconfirmed: UInt8(2)),
            unsignedTransactions: [],
            scanComplete: true,
            metadata: walletMetadataPreview()
        )
        .environment(WalletViewModel(preview: "preview_only"))
    }
}

#Preview("With Unsigned Txns") {
    AsyncPreview {
        TransactionsCardView(
            transactions: transactionsPreviewNew(confirmed: UInt8(3), unconfirmed: UInt8(1)),
            unsignedTransactions: [UnsignedTransaction.previewNew(), UnsignedTransaction.previewNew()],
            scanComplete: true,
            metadata: walletMetadataPreview()
        )
        .environment(WalletViewModel(preview: "preview_only"))
    }
}

#Preview("Amounts in Fiat") {
    var metadata = walletMetadataPreview()
    metadata.fiatOrBtc = .fiat

    return AsyncPreview {
        TransactionsCardView(
            transactions: transactionsPreviewNew(confirmed: UInt8(10), unconfirmed: UInt8(2)),
            unsignedTransactions: [],
            scanComplete: true,
            metadata: metadata
        )
        .environment(WalletViewModel(preview: "preview_only"))
    }
}

#Preview("Sensitive Hidden") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = false

    return
        AsyncPreview {
            TransactionsCardView(
                transactions: transactionsPreviewNew(confirmed: UInt8(10), unconfirmed: UInt8(2)),
                unsignedTransactions: [],
                scanComplete: true,
                metadata: metadata
            )
            .environment(WalletViewModel(preview: "preview_only"))
        }
}

#Preview("Empty") {
    AsyncPreview {
        VStack {
            Text("Test")

            Spacer()
            ScrollView {
                TransactionsCardView(
                    transactions: [],
                    unsignedTransactions: [],
                    scanComplete: true,
                    metadata: walletMetadataPreview()
                )
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
        .environment(WalletViewModel(preview: "preview_only"))
    }
}
