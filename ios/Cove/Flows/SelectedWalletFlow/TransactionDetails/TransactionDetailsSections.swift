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

        TransactionDetailsLockControl(
            lockState: lockState,
            isUpdatingLockState: isUpdatingLockState,
            lockStateLoadError: lockStateLoadError,
            retryLockState: retryLockState,
            toggleLockState: toggleLockState
        )

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
                numberOfConfirmations: numberOfConfirmations
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

        TransactionDetailsLockControl(
            lockState: lockState,
            isUpdatingLockState: isUpdatingLockState,
            lockStateLoadError: lockStateLoadError,
            retryLockState: retryLockState,
            toggleLockState: toggleLockState
        )

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
                numberOfConfirmations: numberOfConfirmations
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
                title: String(localized: "Unable to load lock state"),
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
                    title: lockStateText,
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
        title: String,
        buttonTitle: String,
        systemImage: String,
        isUpdating: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        VStack(spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: systemImage)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)

                Text(title)
                    .font(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(.secondary)
            }

            Button(action: action) {
                HStack(spacing: 6) {
                    if isUpdating {
                        ProgressView()
                            .controlSize(.mini)
                    } else {
                        Image(systemName: systemImage)
                            .font(.footnote.weight(.semibold))
                    }

                    Text(buttonTitle)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
                .padding(.vertical, 8)
                .padding(.horizontal, 12)
                .background(Color.systemGray5)
                .foregroundStyle(.primary)
                .clipShape(Capsule())
                .opacity(isUpdating ? 0.72 : 1)
            }
            .buttonStyle(.plain)
            .disabled(isUpdating)
        }
        .padding(.top, 2)
    }

    private var lockStateText: String {
        switch lockState {
        case .some(.locked):
            String(localized: "Locked")
        case .some(.mixed):
            String(localized: "Mixed")
        case .some(.unlocked):
            String(localized: "Unlocked")
        case .some(.none), nil:
            ""
        }
    }

    private var lockStateButtonText: String {
        switch lockState {
        case .some(.locked):
            String(localized: "Unlock Transaction")
        case .some(.mixed), .some(.unlocked):
            String(localized: "Lock Transaction")
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
