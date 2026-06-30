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
    let lockStateLoadError: String?
    let retryLockState: () -> Void
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

                TransactionDetailsLabelView(details: transactionDetails, manager: manager)
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
    let lockStateLoadError: String?
    let retryLockState: () -> Void
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

                TransactionDetailsLabelView(details: transactionDetails, manager: manager)
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
                lockStateLoadError: lockStateLoadError,
                retryLockState: retryLockState,
                toggleLockState: toggleLockState
            )
        }
    }
}

struct TransactionDetailsLockControl: View {
    let lockState: TransactionLockState?
    let isUpdatingLockState: Bool
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
                    buttonTitle: isUpdatingLockState
                        ? String(localized: "Updating...")
                        : lockStateButtonText,
                    systemImage: lockStateButtonIcon,
                    isUpdating: isUpdatingLockState,
                    action: toggleLockState
                )
            }
        }
    }

    private func content(
        buttonTitle: String,
        systemImage: String,
        isUpdating: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if isUpdating {
                    ProgressView()
                        .controlSize(.mini)
                } else {
                    Image(systemName: systemImage)
                        .font(.caption2.weight(.semibold))
                }

                Text(buttonTitle)
                    .font(.caption)
                    .fontWeight(.semibold)
            }
            .foregroundStyle(Color.secondary.opacity(isUpdating ? 0.72 : 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(isUpdating)
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
}
