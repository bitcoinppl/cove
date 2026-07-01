//
//  TransactionDetailsSections.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/26.
//

import SwiftUI

struct TransactionReceivedDetailsSection: View {
    let transactionDetails: TransactionDetails
    let manager: WalletManager
    let metadata: WalletMetadata
    let numberOfConfirmations: Int?
    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
    let showLockStateUpdatingIndicator: Bool
    let lockStateLoadError: String?
    let retryLockState: () -> Void
    let requestUnlockLockedUtxos: () -> Void
    let toggleLockState: () -> Void

    private var headerIcon: HeaderIcon {
        HeaderIcon(
            isSent: transactionDetails.isSent(),
            isConfirmed: transactionDetails.isConfirmed(),
            numberOfConfirmations: numberOfConfirmations
        )
    }

    var body: some View {
        VStack {
            headerIcon

            VStack(spacing: 4) {
                Text(transactionDetails.isConfirmed() ? "Transaction Received" : "Transaction Pending")
                    .font(.title)
                    .fontWeight(.semibold)
                    .padding(.top, 8)

                TransactionDetailsHeaderLabelRow(
                    transactionDetails: transactionDetails,
                    manager: manager,
                    lockState: lockState,
                    isUpdatingLockState: isUpdatingLockState,
                    requestUnlockLockedUtxos: requestUnlockLockedUtxos
                )
            }
        }

        if transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction was successfully received")
                    .foregroundColor(.secondary)

                Text(transactionDetails.confirmationDateTime() ?? "Unknown")
                    .fontWeight(.semibold)
                    .foregroundColor(.secondary)
            }
            .multilineTextAlignment(.center)
        }

        if !transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction is pending. ")
                    .foregroundColor(.secondary)

                Text("Please check back soon for an update.")
                    .foregroundColor(.secondary)
            }
            .multilineTextAlignment(.center)
        }

        VStack(spacing: 8) {
            Text(transactionDetails.displayAmount(metadata: metadata))
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top, 12)

            AsyncView(
                cachedValue: transactionDetails.amountFiatFmtCached(),
                operation: transactionDetails.amountFiatFmt
            ) { amount in
                Text(amount).foregroundStyle(.primary.opacity(0.8))
            }
        }

        Group {
            if transactionDetails.isConfirmed() {
                TransactionCapsule(text: "Received", icon: "arrow.down.left", color: .statusSuccess)
            } else {
                TransactionCapsule(
                    text: "Receiving",
                    icon: "arrow.down.left",
                    color: .coolGray,
                    textColor: .black.opacity(0.8)
                )
            }
        }
        .padding(.top, 12)

        if let confirmations = numberOfConfirmations, confirmations < 3 {
            VStack {
                Divider().padding(.vertical, 18)
                ConfirmationIndicatorView(current: confirmations)
            }
            .padding(.horizontal, detailsExpandedPadding)
        }

        if metadata.detailsExpanded {
            ReceivedDetailsExpandedView(
                manager: manager,
                transactionDetails: transactionDetails,
                numberOfConfirmations: numberOfConfirmations,
                lockState: lockState,
                isUpdatingLockState: isUpdatingLockState,
                showLockStateUpdatingIndicator: showLockStateUpdatingIndicator,
                lockStateLoadError: lockStateLoadError,
                retryLockState: retryLockState,
                toggleLockState: toggleLockState
            )
        }
    }
}

struct TransactionSentDetailsSection: View {
    let transactionDetails: TransactionDetails
    let manager: WalletManager
    let metadata: WalletMetadata
    let numberOfConfirmations: Int?
    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
    let showLockStateUpdatingIndicator: Bool
    let lockStateLoadError: String?
    let retryLockState: () -> Void
    let requestUnlockLockedUtxos: () -> Void
    let toggleLockState: () -> Void

    private var headerIcon: HeaderIcon {
        HeaderIcon(
            isSent: transactionDetails.isSent(),
            isConfirmed: transactionDetails.isConfirmed(),
            numberOfConfirmations: numberOfConfirmations
        )
    }

    var body: some View {
        VStack {
            headerIcon

            VStack(spacing: 4) {
                Text(transactionDetails.isConfirmed() ? "Transaction Sent" : "Transaction Pending")
                    .font(.title)
                    .fontWeight(.semibold)
                    .padding(.top, 6)

                TransactionDetailsHeaderLabelRow(
                    transactionDetails: transactionDetails,
                    manager: manager,
                    lockState: lockState,
                    isUpdatingLockState: isUpdatingLockState,
                    requestUnlockLockedUtxos: requestUnlockLockedUtxos
                )
            }
        }

        if transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction was sent on")
                    .foregroundColor(.secondary)

                Text(transactionDetails.confirmationDateTime() ?? "Unknown")
                    .fontWeight(.semibold)
                    .foregroundColor(.secondary)
            }
            .multilineTextAlignment(.center)
        }

        if !transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction is pending. ")
                    .foregroundColor(.secondary)

                Text("Please check back soon for an update.")
                    .fontWeight(.semibold)
                    .foregroundColor(.secondary)
            }
            .multilineTextAlignment(.center)
        }

        VStack(spacing: 8) {
            Text(transactionDetails.displayAmount(metadata: metadata))
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top, 12)

            AsyncView(
                cachedValue: transactionDetails.amountFiatFmtCached(),
                operation: transactionDetails.amountFiatFmt
            ) { amount in
                Text(amount).foregroundStyle(.primary.opacity(0.8))
            }
        }

        Group {
            if transactionDetails.isConfirmed() {
                TransactionCapsule(
                    text: "Sent",
                    icon: "arrow.up.right",
                    color: .black,
                    textColor: .white
                )
            } else {
                TransactionCapsule(
                    text: "Sending",
                    icon: "arrow.up.right",
                    color: .coolGray,
                    textColor: .black.opacity(0.8)
                )
            }
        }
        .padding(.top, 12)

        if let confirmations = numberOfConfirmations, confirmations < 3 {
            VStack {
                Divider().padding(.vertical, 18)
                ConfirmationIndicatorView(current: confirmations)
            }
            .padding(.horizontal, detailsExpandedPadding)
        }

        if metadata.detailsExpanded {
            SentDetailsExpandedView(
                manager: manager,
                transactionDetails: transactionDetails,
                numberOfConfirmations: numberOfConfirmations,
                lockState: lockState,
                isUpdatingLockState: isUpdatingLockState,
                showLockStateUpdatingIndicator: showLockStateUpdatingIndicator,
                lockStateLoadError: lockStateLoadError,
                retryLockState: retryLockState,
                toggleLockState: toggleLockState
            )
        }
    }
}

private struct TransactionDetailsHeaderLabelRow: View {
    let transactionDetails: TransactionDetails
    let manager: WalletManager
    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
    let requestUnlockLockedUtxos: () -> Void

    private var lockedUtxosState: TransactionLockState? {
        guard let lockState, lockState.showsCollapsedLockTreatment else { return nil }

        return lockState
    }

    var body: some View {
        if let lockedUtxosState {
            HStack(spacing: 12) {
                TransactionDetailsLabelView(details: transactionDetails, manager: manager)
                    .lineLimit(1)

                TransactionCollapsedLockBadge(
                    lockState: lockedUtxosState,
                    isUpdatingLockState: isUpdatingLockState,
                    requestUnlockLockedUtxos: requestUnlockLockedUtxos
                )
                .fixedSize(horizontal: true, vertical: false)
            }
            .padding(.horizontal, detailsExpandedPadding)
        } else {
            TransactionDetailsLabelView(details: transactionDetails, manager: manager)
        }
    }
}

private struct TransactionCollapsedLockBadge: View {
    let lockState: TransactionLockState
    let isUpdatingLockState: Bool
    let requestUnlockLockedUtxos: () -> Void

    var body: some View {
        Button(action: requestUnlockLockedUtxos) {
            Label {
                Text(lockState.collapsedLockBadgeTitle)
                    .font(.caption)
                    .fontWeight(.semibold)
                    .lineLimit(1)
            } icon: {
                Image(systemName: "lock.fill")
                    .font(.caption.weight(.semibold))
            }
            .foregroundStyle(Color.statusWarning)
            .padding(.horizontal, 9)
            .padding(.vertical, 5)
            .background(Color.statusWarning.opacity(0.14))
            .clipShape(Capsule())
            .opacity(isUpdatingLockState ? 0.72 : 1)
        }
        .buttonStyle(.plain)
        .disabled(isUpdatingLockState)
        .accessibilityLabel(lockState.collapsedLockBadgeTitle)
    }
}

struct TransactionDetailsLockControl: View {
    private static let buttonMinWidth: CGFloat = 82
    private static let symbolFrameSize: CGFloat = 14

    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
    let showLockStateUpdatingIndicator: Bool
    let lockStateLoadError: String?
    let retryLockState: () -> Void
    let toggleLockState: () -> Void

    var body: some View {
        if lockStateLoadError != nil {
            content(
                buttonTitle: String(localized: "Retry"),
                systemImage: "arrow.clockwise",
                action: retryLockState
            )
        } else {
            switch lockState {
            case .some(.none), nil:
                EmptyView()
            case .some(.unlocked), .some(.locked), .some(.mixed):
                content(
                    buttonTitle: showLockStateUpdatingIndicator
                        ? String(localized: "Updating...")
                        : lockStateButtonText,
                    systemImage: lockStateButtonIcon,
                    isDisabled: isUpdatingLockState,
                    showsUpdatingIndicator: showLockStateUpdatingIndicator,
                    action: toggleLockState
                )
            }
        }
    }

    private func content(
        buttonTitle: String,
        systemImage: String,
        isDisabled: Bool = false,
        showsUpdatingIndicator: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if showsUpdatingIndicator {
                    ProgressView()
                        .controlSize(.mini)
                        .frame(
                            width: Self.symbolFrameSize,
                            height: Self.symbolFrameSize
                        )
                } else {
                    Image(systemName: systemImage)
                        .font(.caption2.weight(.semibold))
                        .frame(
                            width: Self.symbolFrameSize,
                            height: Self.symbolFrameSize
                        )
                }

                Text(buttonTitle)
                    .font(.caption)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                    .contentTransition(.identity)
            }
            .frame(minWidth: Self.buttonMinWidth, alignment: .leading)
            .foregroundStyle(actionColor.opacity(isDisabled ? 0.72 : 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(isDisabled)
        .transaction { transaction in
            transaction.animation = nil
        }
    }

    private var lockStateButtonText: String {
        switch lockState {
        case .some(.locked):
            String(localized: "Unlock")
        case .some(.mixed), .some(.unlocked):
            String(localized: "Lock")
        case .some(.none), nil:
            ""
        }
    }

    private var lockStateButtonIcon: String {
        switch lockState {
        case .some(.locked):
            "lock.open"
        case .some(.mixed), .some(.unlocked):
            "lock"
        case .some(.none), nil:
            "lock"
        }
    }

    private var actionColor: Color {
        switch lockState {
        case .some(.locked):
            .systemRed
        case .some(.mixed), .some(.unlocked), .some(.none), nil:
            .secondary
        }
    }
}

private extension TransactionLockState {
    var showsCollapsedLockTreatment: Bool {
        self == .locked || self == .mixed
    }

    var collapsedLockBadgeTitle: String {
        switch self {
        case .locked:
            String(localized: "UTXOs locked")
        case .mixed:
            String(localized: "Some UTXOs locked")
        case .none, .unlocked:
            ""
        }
    }
}
