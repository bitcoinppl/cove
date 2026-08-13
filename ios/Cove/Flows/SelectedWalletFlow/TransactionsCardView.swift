//
//  TransactionsCardView.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import MijickPopups
import SwiftUI

private let scrollThresholdIndex = 5

enum TransactionsCopy {
    static var checkingWalletHistory: String {
        String(localized: "Checking wallet history")
    }
}

struct TransactionsCardView: View {
    @Environment(AppManager.self) var app
    @Environment(WalletManager.self) var manager

    let transactions: [CoveCore.Transaction]
    let unsignedTransactions: [UnsignedTransaction]
    let metadata: WalletMetadata

    private let screenHeight = UIScreen.main.bounds.height
    private var scanProgress: WalletScanProgress? {
        if case let .scanning(progress) = manager.scanStatus {
            return progress
        }

        return nil
    }

    private var isScanning: Bool {
        // keep all sources so reconcile message ordering cannot hide active scanning
        isWalletLoading || manager.ledgerState.initialScanActive || manager.scanStatus.isActive
    }

    private var isWalletLoading: Bool {
        if case .loading = manager.loadState {
            return true
        }

        return false
    }

    private var scanSpinnerMessage: String? {
        if isWalletLoading {
            return TransactionsCopy.checkingWalletHistory
        }

        return manager.ledgerState.initialScanComplete ? nil : TransactionsCopy.checkingWalletHistory
    }

    private var isScanProgressVisible: Bool {
        scanProgress != nil
    }

    private var scanProgressFraction: Double {
        Double(scanProgress?.progressBasisPoints ?? 0) / 10000
    }

    var body: some View {
        VStack {
            TransactionsCardContent(
                transactions: transactions,
                unsignedTransactions: unsignedTransactions,
                metadata: metadata,
                isScanning: isScanning,
                scanProgress: scanProgress,
                isScanProgressVisible: isScanProgressVisible,
                scanProgressFraction: scanProgressFraction,
                scanSpinnerMessage: scanSpinnerMessage,
                screenHeight: screenHeight
            )
            .padding()
            .padding(.top, 5)
        }
        .onDisappear(perform: dismissPopups)
    }

    private func dismissPopups() {
        Task { await dismissAllPopups() }
    }
}

private struct TransactionsCardContent: View {
    let transactions: [CoveCore.Transaction]
    let unsignedTransactions: [UnsignedTransaction]
    let metadata: WalletMetadata
    let isScanning: Bool
    let scanProgress: WalletScanProgress?
    let isScanProgressVisible: Bool
    let scanProgressFraction: Double
    let scanSpinnerMessage: String?
    let screenHeight: CGFloat

    var body: some View {
        VStack {
            HStack {
                Text("Transactions")
                    .foregroundStyle(.secondary)
                    .font(.subheadline)
                    .fontWeight(.bold)
                Spacer()
            }
            .padding(.bottom, 12)

            if isScanning, !transactions.isEmpty || !unsignedTransactions.isEmpty {
                TransactionsScanningStrip(
                    isProgressVisible: isScanProgressVisible,
                    progressFraction: scanProgressFraction,
                    spinnerMessage: scanSpinnerMessage
                )
            }

            TransactionRows(
                transactions: transactions,
                unsignedTransactions: unsignedTransactions,
                metadata: metadata
            )

            TransactionsEmptyContent(
                transactionsAreEmpty: transactions.isEmpty,
                unsignedTransactionsAreEmpty: unsignedTransactions.isEmpty,
                isScanning: isScanning,
                scanProgress: scanProgress,
                isScanProgressVisible: isScanProgressVisible,
                scanProgressFraction: scanProgressFraction,
                scanSpinnerMessage: scanSpinnerMessage,
                screenHeight: screenHeight
            )
        }
    }
}

private struct TransactionsScanningStrip: View {
    let isProgressVisible: Bool
    let progressFraction: Double
    let spinnerMessage: String?

    var body: some View {
        Group {
            if isProgressVisible {
                TransactionsScanProgressStrip(progressFraction: progressFraction)
            } else {
                TransactionsScanSpinnerStrip(message: spinnerMessage)
            }
        }
        .padding(.bottom, 10)
    }
}

private struct TransactionRows: View {
    let transactions: [CoveCore.Transaction]
    let unsignedTransactions: [UnsignedTransaction]
    let metadata: WalletMetadata

    var body: some View {
        LazyVStack(alignment: .leading) {
            UnsignedTransactionRows(
                transactions: unsignedTransactions,
                metadata: metadata
            )

            ForEach(Array(transactions.enumerated()), id: \.element.id) { index, transaction in
                TransactionRow(
                    txn: transaction,
                    metadata: metadata,
                    index: unsignedTransactions.count + index
                )
                .id(transaction.id.description)
            }
        }
    }
}

private struct UnsignedTransactionRows: View {
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var manager

    let transactions: [UnsignedTransaction]
    let metadata: WalletMetadata

    var body: some View {
        ForEach(Array(transactions.enumerated()), id: \.element.id) { index, transaction in
            VStack(alignment: .leading) {
                UnsignedTransactionView(
                    txn: transaction,
                    metadata: metadata,
                    index: index
                )
                .contentShape(
                    .contextMenuPreview,
                    RoundedRectangle(cornerRadius: 8)
                        .inset(by: -6)
                )
                .contextMenu {
                    Button(role: .destructive) {
                        delete(transaction)
                    } label: {
                        Label("Delete", systemImage: "trash")
                    }
                }
                .padding(.vertical, 6)

                Divider().opacity(0.7)
            }
            .id(transaction.id().description)
        }
    }

    private func delete(_ transaction: UnsignedTransaction) {
        do {
            try manager.rust.deleteUnsignedTransaction(txId: transaction.id())
        } catch {
            Log.error("Failed to delete unsigned transaction \(transaction.id()): \(error)")
            app.alertState = .init(.general(
                title: "Delete Failed",
                message: "Unable to delete transaction: \(error.localizedDescription)"
            ))
        }
    }
}

private struct TransactionsEmptyContent: View {
    let transactionsAreEmpty: Bool
    let unsignedTransactionsAreEmpty: Bool
    let isScanning: Bool
    let scanProgress: WalletScanProgress?
    let isScanProgressVisible: Bool
    let scanProgressFraction: Double
    let scanSpinnerMessage: String?
    let screenHeight: CGFloat

    var body: some View {
        if transactionsAreEmpty, unsignedTransactionsAreEmpty, isScanning {
            VStack {
                if isScanProgressVisible {
                    EmptyWalletScanState(
                        scanProgress: scanProgress,
                        progressFraction: scanProgressFraction
                    )
                    .frame(maxWidth: .infinity)
                    .padding(.top, 56)
                } else {
                    EmptyWalletScanSpinnerState(message: scanSpinnerMessage)
                        .frame(maxWidth: .infinity)
                        .padding(.top, 56)
                }

                Spacer()
                    .frame(minHeight: screenHeight * 0.2)
            }
        } else if transactionsAreEmpty, unsignedTransactionsAreEmpty {
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
}

struct TransactionsScanSpinnerStrip: View {
    let message: String?

    init(message: String? = nil) {
        self.message = message
    }

    var body: some View {
        HStack(spacing: 8) {
            ProgressView()
                .scaleEffect(0.75)
                .tint(.secondary)

            if let message {
                Text(message)
                    .foregroundStyle(.secondary.opacity(0.7))
                    .font(.caption2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .center)
        .frame(height: 18)
    }
}

struct TransactionsScanProgressStrip: View {
    let progressFraction: Double

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            ProgressView(value: progressFraction)
                .progressViewStyle(.linear)
                .tint(.primary.opacity(0.45))
                .frame(height: 2)

            Text("Scanning for transactions...")
                .foregroundStyle(.secondary.opacity(0.7))
                .font(.caption2)
        }
    }
}

struct EmptyWalletScanSpinnerState: View {
    let message: String?

    init(message: String? = nil) {
        self.message = message
    }

    var body: some View {
        VStack(spacing: 10) {
            ProgressView()
                .tint(.primary)

            if let message {
                Text(message)
                    .foregroundStyle(.secondary)
                    .font(.body)
            }
        }
    }
}

struct EmptyWalletScanState: View {
    let scanProgress: WalletScanProgress?
    let progressFraction: Double

    var body: some View {
        VStack(spacing: 10) {
            Text(TransactionsCopy.checkingWalletHistory)
                .foregroundStyle(.secondary)
                .font(.body)

            ProgressView(value: progressFraction)
                .progressViewStyle(.linear)
                .tint(.primary)
                .frame(maxWidth: 260)

            Text("\(scanProgress?.checked ?? 0) addresses checked")
                .foregroundStyle(.secondary)
                .font(.caption)
        }
    }
}

private func showsLockedTransactionTreatment(_ lockState: TransactionLockState?) -> Bool {
    lockState == .locked || lockState == .mixed
}

private func amountColor(_ direction: TransactionDirection, locked: Bool = false) -> Color {
    switch direction {
    case .incoming:
        locked ? .green.opacity(0.72) : .green
    case .outgoing:
        locked ? .secondary.opacity(0.75) : .primary.opacity(0.8)
    }
}

struct TransactionRow: View {
    @Environment(WalletManager.self) var manager
    var txn: CoveCore.Transaction
    var metadata: WalletMetadata
    var index: Int

    var body: some View {
        VStack(alignment: .leading) {
            Group {
                switch txn {
                case let .confirmed(txn):
                    ConfirmedTransactionView(txn: txn, metadata: metadata, index: index)
                case let .unconfirmed(txn):
                    UnconfirmedTransactionView(txn: txn, metadata: metadata, index: index)
                }
            }
            .padding(.vertical, 6)

            Divider().opacity(0.7)
        }
    }
}

struct ConfirmedTransactionView: View {
    @Environment(\.navigate) private var navigate
    @Environment(WalletManager.self) var manager

    let txn: ConfirmedTransaction
    let metadata: WalletMetadata
    let index: Int

    private var lockState: TransactionLockState? {
        manager.transactionLockStates[txn.id()]
    }

    private var amount: String {
        if case .btc = metadata.fiatOrBtc {
            return privateShow(
                manager.displaySentAndReceivedAmount(txn.sentAndReceived())
            )
        }

        // fiat
        guard let fiatAmount = txn.fiatAmount() else { return privateShow("---") }
        return privateShow(
            manager.displayFiatAmountWithDirection(
                fiatAmount.amount,
                direction: txn.sentAndReceived().direction()
            )
        )
    }

    private var secondaryAmount: String {
        if case .btc = metadata.fiatOrBtc {
            // primary is BTC, secondary is fiat
            guard let fiatAmount = txn.fiatAmount() else { return privateShow("---") }
            return privateShow(
                manager.displayFiatAmountWithDirection(
                    fiatAmount.amount,
                    direction: txn.sentAndReceived().direction()
                )
            )
        }

        // primary is fiat, secondary is BTC/sats
        return privateShow(
            manager.displaySentAndReceivedAmount(txn.sentAndReceived())
        )
    }

    private func privateShow(_ text: String, placeholder: String = "••••••") -> String {
        if !metadata.sensitiveVisible {
            placeholder
        } else {
            text
        }
    }

    private func goToTransactionDetails() {
        let txId = txn.id()
        if index > scrollThresholdIndex { manager.scrolledTransactionId = txId.description }
        navigate(Route.transactionDetails(id: metadata.id, txId: txId))
    }

    private func refreshLockState() async {
        do {
            _ = try await manager.transactionLockState(for: txn.id())
        } catch {
            Log.error("Failed to load transaction lock state for \(txn.id()): \(error)")
            manager.clearTransactionLockState(for: txn.id())
        }
    }

    var label: String {
        if metadata.showLabels { return txn.label() }
        return txn.sentAndReceived().label()
    }

    var body: some View {
        let direction = txn.sentAndReceived().direction()
        let showsLockedTreatment = showsLockedTransactionTreatment(lockState)

        HStack {
            TxnIcon(direction: direction, locked: showsLockedTreatment)

            VStack(alignment: .leading, spacing: 5) {
                Text(label)
                    .font(.subheadline)
                    .fontWeight(.medium)
                    .foregroundColor(showsLockedTreatment ? .secondary.opacity(0.72) : .primary.opacity(0.65))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .minimumScaleFactor(0.90)
                    .padding(.trailing, 10)

                Text(privateShow(txn.confirmedAtFmt()))
                    .font(.caption)
                    .foregroundColor(showsLockedTreatment ? .secondary.opacity(0.68) : .secondary)
            }

            Spacer()

            VStack(alignment: .trailing) {
                Text(amount)
                    .foregroundStyle(amountColor(direction, locked: showsLockedTreatment))
                    .contentTransition(.numericText())

                Text(secondaryAmount)
                    .font(.caption)
                    .foregroundColor(showsLockedTreatment ? .secondary.opacity(0.62) : .secondary)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            goToTransactionDetails()
        }
        .task(id: txn.id().description) {
            await refreshLockState()
        }
    }
}

struct UnconfirmedTransactionView: View {
    @Environment(\.navigate) private var navigate
    @Environment(WalletManager.self) var manager

    let txn: UnconfirmedTransaction
    let metadata: WalletMetadata
    let index: Int

    private var lockState: TransactionLockState? {
        manager.transactionLockStates[txn.id()]
    }

    func privateShow(_ text: String, placeholder: String = "••••••") -> String {
        if !metadata.sensitiveVisible {
            placeholder
        } else {
            text
        }
    }

    private var amount: String {
        if case .btc = metadata.fiatOrBtc {
            return privateShow(
                manager.displaySentAndReceivedAmount(txn.sentAndReceived())
            )
        }

        // fiat
        if let fiatAmount = txn.fiatAmount() {
            return privateShow(
                manager.displayFiatAmountWithDirection(
                    fiatAmount.amount,
                    direction: txn.sentAndReceived().direction()
                )
            )
        } else {
            return privateShow("---")
        }
    }

    private var secondaryAmount: String {
        if case .btc = metadata.fiatOrBtc {
            // primary is BTC, secondary is fiat
            if let fiatAmount = txn.fiatAmount() {
                return privateShow(
                    manager.displayFiatAmountWithDirection(
                        fiatAmount.amount,
                        direction: txn.sentAndReceived().direction()
                    )
                )
            } else {
                return privateShow("---")
            }
        }

        // primary is fiat, secondary is BTC/sats
        return privateShow(
            manager.displaySentAndReceivedAmount(txn.sentAndReceived())
        )
    }

    private func goToTransactionDetails() {
        let txId = txn.id()
        if index > scrollThresholdIndex { manager.scrolledTransactionId = txId.description }
        navigate(Route.transactionDetails(id: metadata.id, txId: txId))
    }

    private func refreshLockState() async {
        do {
            _ = try await manager.transactionLockState(for: txn.id())
        } catch {
            Log.error("Failed to load transaction lock state for \(txn.id()): \(error)")
            manager.clearTransactionLockState(for: txn.id())
        }
    }

    var body: some View {
        let direction = txn.sentAndReceived().direction()
        let showsLockedTreatment = showsLockedTransactionTreatment(lockState)

        HStack {
            TxnIcon(
                direction: direction,
                confirmed: false,
                locked: showsLockedTreatment
            )
            .opacity(showsLockedTreatment ? 1 : 0.6)

            VStack(alignment: .leading, spacing: 5) {
                Text(txn.label())
                    .font(.subheadline)
                    .fontWeight(.medium)
                    .foregroundColor(showsLockedTreatment ? .secondary.opacity(0.72) : .primary.opacity(0.4))
            }
            Spacer()
            VStack(alignment: .trailing) {
                Text(amount)
                    .foregroundStyle(
                        amountColor(direction, locked: showsLockedTreatment)
                            .opacity(showsLockedTreatment && direction == .incoming ? 1 : 0.65)
                    )

                Text(secondaryAmount)
                    .font(.caption)
                    .foregroundColor(showsLockedTreatment ? .secondary.opacity(0.62) : .secondary)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture {
            goToTransactionDetails()
        }
        .task(id: txn.id().description) {
            await refreshLockState()
        }
    }
}

struct UnsignedTransactionView: View {
    @Environment(\.navigate) private var navigate
    @Environment(WalletManager.self) var manager
    @Environment(\.colorScheme) var colorScheme

    // args
    let txn: UnsignedTransaction
    let metadata: WalletMetadata
    let index: Int

    /// private
    @State private var fiatAmount: Double? = nil

    func privateShow(_ text: String, placeholder: String = "••••••") -> String {
        if !metadata.sensitiveVisible {
            placeholder
        } else {
            text
        }
    }

    private var amount: String {
        // btc or sats (unsigned transactions are always outgoing)
        if case .btc = metadata.fiatOrBtc {
            return privateShow(
                manager.displayAmountWithDirection(txn.spendingAmount(), direction: .outgoing)
            )
        }

        // fiat
        guard let fiatAmount else { return privateShow("---") }
        return privateShow(
            manager.displayFiatAmountWithDirection(fiatAmount, direction: .outgoing)
        )
    }

    private var secondaryAmount: String {
        if case .btc = metadata.fiatOrBtc {
            // primary is BTC, secondary is fiat
            guard let fiatAmount else { return privateShow("---") }
            return privateShow(
                manager.displayFiatAmountWithDirection(fiatAmount, direction: .outgoing)
            )
        }

        // primary is fiat, secondary is BTC/sats
        return privateShow(
            manager.displayAmountWithDirection(txn.spendingAmount(), direction: .outgoing)
        )
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

                Text(secondaryAmount)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .task {
            fiatAmount = manager.amountInFiatCached(txn.spendingAmount())
        }
        .contentShape(Rectangle())
        .onTapGesture {
            if index > scrollThresholdIndex { manager.scrolledTransactionId = txn.id().description }

            let hardwareExportRoute =
                RouteFactory().sendHardwareExport(
                    id: metadata.id,
                    details: txn.details()
                )

            navigate(hardwareExportRoute)
        }
    }
}

private struct TxnIcon: View {
    @Environment(\.colorScheme) var colorScheme

    let direction: TransactionDirection
    var confirmed: Bool = true
    var locked: Bool = false

    var iconColor: Color {
        if locked { return .systemGray5 }

        return colorScheme == .dark ? Color.gray.opacity(0.35) : Color.primary.opacity(0.75)
    }

    var iconForeground: Color {
        locked ? .secondary : .white
    }

    var arrow: String {
        if locked {
            return "lock.fill"
        }

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
        ZStack {
            Image(systemName: arrow)
                .foregroundColor(iconForeground)
                .padding()
        }
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
            metadata: walletMetadataPreview()
        )
        .environment(WalletManager(preview: .only))
    }
}

#Preview("Full of Txns - Scanning") {
    AsyncPreview {
        ScrollView {
            TransactionsCardView(
                transactions: transactionsPreviewNew(confirmed: UInt8(10), unconfirmed: UInt8(1)),
                unsignedTransactions: [],
                metadata: walletMetadataPreview()
            )
            .background(.thickMaterial)
            .environment(WalletManager(preview: .only))
        }
    }
}

#Preview("Empty - Scanning") {
    AsyncPreview {
        TransactionsCardView(
            transactions: [],
            unsignedTransactions: [],
            metadata: walletMetadataPreview()
        )
        .environment(WalletManager(preview: .only))
    }
}

#Preview("With Unconfirmed Txns") {
    AsyncPreview {
        TransactionsCardView(
            transactions: transactionsPreviewNew(confirmed: UInt8(10), unconfirmed: UInt8(2)),
            unsignedTransactions: [],
            metadata: walletMetadataPreview()
        )
        .environment(WalletManager(preview: .only))
    }
}

#Preview("With Unsigned Txns") {
    AsyncPreview {
        TransactionsCardView(
            transactions: transactionsPreviewNew(confirmed: UInt8(3), unconfirmed: UInt8(1)),
            unsignedTransactions: [
                UnsignedTransaction.previewNew(), UnsignedTransaction.previewNew(),
            ],
            metadata: walletMetadataPreview()
        )
        .environment(WalletManager(preview: .only))
    }
}

#Preview("Amounts in Fiat") {
    var metadata = walletMetadataPreview()
    metadata.fiatOrBtc = .fiat

    return AsyncPreview {
        TransactionsCardView(
            transactions: transactionsPreviewNew(confirmed: UInt8(10), unconfirmed: UInt8(2)),
            unsignedTransactions: [],
            metadata: metadata
        )
        .environment(WalletManager(preview: .only))
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
                metadata: metadata
            )
            .environment(WalletManager(preview: .only))
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
        .environment(WalletManager(preview: .only))
    }
}
