//
//  TransactionRowMenu.swift
//  Cove
//
//  Created by Praveen Perera on 11/23/26.
//

import MijickPopups
import SwiftUI

extension EnvironmentValues {
    /// Opens the bump fee sheet for a pending send, set by the screen that hosts the sheet
    @Entry var openFeeBump: (TxId) -> Void = { _ in }
}

extension PendingSendStatus {
    /// The row's state for UI tests, which can't see its colors
    var rowAccessibilityValue: String {
        switch self {
        case .recent: "recent"
        case .waiting: "waiting"
        case .delayed: "delayed"
        }
    }
}

/// Long-press menu of a transaction row, with a card that previews the transaction
///
/// Speed Up leads, only for a pending send the wallet can bump
struct TransactionRowContextMenu: ViewModifier {
    @Environment(\.navigate) private var navigate
    @Environment(\.openFeeBump) private var openFeeBump
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var manager

    let txn: CoveCore.Transaction
    let metadata: WalletMetadata
    let index: Int

    private var isBumpable: Bool {
        guard case let .unconfirmed(txn) = txn else { return false }
        return txn.pendingSend()?.isBumpable() ?? false
    }

    func body(content: Content) -> some View {
        content.contextMenu {
            if isBumpable {
                Button {
                    openFeeBump(txn.id)
                } label: {
                    Label {
                        Text("Speed Up")
                        Text("Get your transaction confirmed faster")
                    } icon: {
                        Image(systemName: "bolt.fill")
                    }
                }
            }

            Button("Copy Address", systemImage: "doc.on.doc", action: copyAddress)
            Button("Copy Transaction ID", systemImage: "number", action: copyTransactionId)
            Button("View Transaction Details", systemImage: "info.circle", action: viewDetails)
        } preview: {
            TransactionRowPreviewCard(txn: txn, metadata: metadata, manager: manager, app: app)
        }
    }

    private func copyAddress() {
        let txId = txn.id

        Task {
            do {
                let presentation = try await manager.transactionDetails(for: txId)
                guard let address = presentation.details().address() else { return }

                await MainActor.run { UIPasteboard.general.string = address.unformatted() }
                await FloaterPopup(text: "Address Copied").dismissAfter(2).present()
            } catch {
                Log.error("Unable to copy the address of \(txId): \(error)")
            }
        }
    }

    private func copyTransactionId() {
        UIPasteboard.general.string = txn.id.description
        Task { await FloaterPopup(text: "Transaction ID Copied").dismissAfter(2).present() }
    }

    private func viewDetails() {
        let txId = txn.id
        if index > 5 { manager.scrolledTransactionId = txId.description }
        navigate(Route.transactionDetails(id: metadata.id, txId: txId))
    }
}

/// A pending send's "Unconfirmed · 32m" under its label, orange once it is delayed
struct PendingSendSubtitle: View {
    let pendingSend: PendingSendStatus

    var body: some View {
        // elapsed time counts to now, so read it again every minute
        TimelineView(.everyMinute) { _ in
            if let elapsed = pendingSend.elapsedFmt() {
                Text("Unconfirmed · \(elapsed)")
                    .font(.caption)
                    .italic(pendingSend.isDelayed())
                    .foregroundStyle(pendingSend.isDelayed() ? Color.orange : Color.secondary)
                    .accessibilityIdentifier("transactionRow.pendingStatus")
            }
        }
    }
}

/// The icon of a delayed send, which opens the "Taking longer than usual" popup
struct DelayedTransactionIcon: View {
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var manager
    @Environment(\.openFeeBump) private var openFeeBump

    let txId: TxId
    let pendingSend: PendingSendStatus

    var body: some View {
        Button(action: showPopup) {
            FeeBumpDelayedIcon()
        }
        .buttonStyle(.plain)
        .padding(.trailing, 5)
        .accessibilityLabel("Taking longer than usual")
        .accessibilityIdentifier("transactionRow.delayedIcon")
    }

    private func showPopup() {
        let popup = FeeBumpDelayedPopup(
            app: app,
            walletManager: manager,
            txId: txId,
            pendingSend: pendingSend,
            customize: { [openFeeBump, txId] in openFeeBump(txId) }
        )

        Task { await popup.present() }
    }
}

/// What a long press shows of a transaction above its menu: its status, amount, date, address
/// and confirmations
private struct TransactionRowPreviewCard: View {
    let txn: CoveCore.Transaction
    let metadata: WalletMetadata
    let manager: WalletManager
    let app: AppManager

    @State private var details: TransactionDetailsPresentation?

    private var pendingSend: PendingSendStatus? {
        guard case let .unconfirmed(txn) = txn else { return nil }
        return txn.pendingSend()
    }

    private var isConfirmed: Bool {
        if case .confirmed = txn { return true }
        return false
    }

    private var sentAndReceived: SentAndReceived {
        switch txn {
        case let .confirmed(txn): txn.sentAndReceived()
        case let .unconfirmed(txn): txn.sentAndReceived()
        }
    }

    private var label: String {
        switch txn {
        case let .confirmed(txn): metadata.showLabels ? txn.label() : txn.sentAndReceived().label()
        case let .unconfirmed(txn): txn.label()
        }
    }

    private var fiatAmount: FiatAmount? {
        switch txn {
        case let .confirmed(txn): txn.fiatAmount()
        case let .unconfirmed(txn): txn.fiatAmount()
        }
    }

    private var date: String {
        switch txn {
        case let .confirmed(txn):
            return txn.confirmedAtFmt()
        case let .unconfirmed(txn):
            let firstSeen = Date(timeIntervalSince1970: TimeInterval(txn.firstSeen()))
            return firstSeen.formatted(date: .abbreviated, time: .shortened)
        }
    }

    private var confirmations: String {
        guard let count = details?.confirmations() else { return isConfirmed ? "Confirmed" : "0 · Pending" }
        return "\(count) · Confirmed"
    }

    private func privateShow(_ text: String) -> String {
        metadata.sensitiveVisible ? text : "••••••"
    }

    private var amount: String {
        privateShow(manager.displaySentAndReceivedAmount(sentAndReceived))
    }

    private var fiat: String? {
        fiatAmount.map {
            privateShow(manager.displayFiatAmountWithDirection($0.amount, direction: sentAndReceived.direction()))
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            TransactionRowPreviewHeader(label: label, pendingSend: pendingSend, amount: amount, fiat: fiat) {
                TransactionRowPreviewIcon(
                    pendingSend: pendingSend,
                    direction: sentAndReceived.direction(),
                    isConfirmed: isConfirmed,
                    app: app
                )
            }

            previewField("Date", privateShow(date))

            if let address = details?.details().address() {
                previewField("Address", privateShow(address.unformatted().addressSpacedOut()))
                    .monospaced()
            }

            TransactionRowPreviewConfirmations(isConfirmed: isConfirmed, text: confirmations)
        }
        .padding(22)
        .frame(width: 320, alignment: .leading)
        .background(Color.coveBg)
        .task { await loadDetails() }
    }

    private func previewField(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.footnote)
                .foregroundStyle(.secondary)

            Text(value)
                .font(.subheadline)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func loadDetails() async {
        details = try? await manager.transactionDetails(for: txn.id)
    }
}

private struct TransactionRowPreviewHeader<Icon: View>: View {
    let label: String
    let pendingSend: PendingSendStatus?
    let amount: String
    let fiat: String?
    @ViewBuilder let icon: Icon

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            icon

            VStack(alignment: .leading, spacing: 4) {
                Text(label)
                    .font(.headline)

                if let pendingSend {
                    PendingSendSubtitle(pendingSend: pendingSend)
                }

                Text(amount)
                    .font(.title3)
                    .fontWeight(.semibold)
                    .padding(.top, 6)

                if let fiat {
                    Text(fiat)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

/// A delayed send's orange clock, or the usual direction icon
private struct TransactionRowPreviewIcon: View {
    let pendingSend: PendingSendStatus?
    let direction: TransactionDirection
    let isConfirmed: Bool
    let app: AppManager

    var body: some View {
        if let pendingSend, pendingSend.isDelayed() {
            FeeBumpDelayedIcon(size: 44)
        } else {
            TxnIcon(direction: direction, confirmed: isConfirmed)
                .environment(app)
        }
    }
}

private struct TransactionRowPreviewConfirmations: View {
    let isConfirmed: Bool
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Confirmations")
                .font(.footnote)
                .foregroundStyle(.secondary)

            HStack(spacing: 6) {
                Circle()
                    .fill(isConfirmed ? Color.green : Color.orange)
                    .frame(width: 7, height: 7)

                Text(text)
                    .font(.subheadline)
            }
        }
    }
}
