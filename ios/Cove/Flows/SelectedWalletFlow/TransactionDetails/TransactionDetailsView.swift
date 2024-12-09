//
//  TransactionDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct TransactionDetailsView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.openURL) private var openURL
    private let screenWidth = UIScreen.main.bounds.width
    private let screenHeight = UIScreen.main.bounds.height

    @State private var numberOfConfirmations: Int? = nil
    @State private var scrollPosition = ScrollPosition()

    // public
    let id: WalletId
    let transactionDetails: TransactionDetails
    var manager: WalletManager

    var headerIcon: HeaderIcon {
        HeaderIcon(
            isSent: transactionDetails.isSent(),
            isConfirmed: transactionDetails.isConfirmed(),
            numberOfConfirmations: numberOfConfirmations
        )
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var detailsExpanded: Bool {
        metadata.detailsExpanded
    }

    @ViewBuilder
    var ReceivedDetails: some View {
        Text(transactionDetails.isConfirmed() ? "Transaction Received" : "Transaction Pending")
            .font(.title)
            .fontWeight(.semibold)
            .padding(.top, 8)

        // confirmed
        if transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction was successfully received")
                    .foregroundColor(.gray)

                Text(transactionDetails.confirmationDateTime() ?? "Unknown")
                    .fontWeight(.semibold)
                    .foregroundColor(.gray)
            }
            .multilineTextAlignment(.center)
        }

        // pending
        if !transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction is pending. ")
                    .foregroundColor(.gray)

                Text("Please check back soon for an update.")
                    .foregroundColor(.gray)
            }
            .multilineTextAlignment(.center)
        }

        Text(manager.rust.displayAmount(amount: transactionDetails.amount()))
            .font(.largeTitle)
            .fontWeight(.bold)
            .padding(.top, 12)

        AsyncView(operation: transactionDetails.amountFiatFmt) { amount in
            Text("≈ $\(amount) USD").foregroundStyle(.primary.opacity(0.8))
        }

        Group {
            if transactionDetails.isConfirmed() {
                TransactionCapsule(text: "Received", icon: "arrow.down.left", color: .green)
            } else {
                TransactionCapsule(
                    text: "Receiving", icon: "arrow.down.left",
                    color: .coolGray, textColor: .black.opacity(0.8)
                )
            }
        }
        .padding(.top, 12)

        // confirmations pills
        if let confirmations = numberOfConfirmations, confirmations < 3 {
            VStack {
                Divider().padding(.vertical, 18)
                ConfirmationIndicatorView(current: confirmations)
            }
            .padding(.horizontal, detailsExpandedPadding)
        }

        // MARK: Received Details Expanded

        if metadata.detailsExpanded {
            ReceivedDetailsExpandedView(
                manager: manager, transactionDetails: transactionDetails,
                numberOfConfirmations: numberOfConfirmations
            )
        }
    }

    @ViewBuilder
    var SentDetails: some View {
        Text(transactionDetails.isConfirmed() ? "Transaction Sent" : "Transaction Pending")
            .font(.title)
            .fontWeight(.semibold)
            .padding(.top, 6)

        // confirmed
        if transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction was sent on")
                    .foregroundColor(.gray)

                Text(transactionDetails.confirmationDateTime() ?? "Unknown")
                    .foregroundColor(.gray)
            }
            .multilineTextAlignment(.center)
        }

        // pending
        if !transactionDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction is pending. ")
                    .foregroundColor(.gray)

                Text("Please check back soon for an update.")
                    .fontWeight(.semibold)
                    .foregroundColor(.gray)
            }
            .multilineTextAlignment(.center)
        }

        Text(manager.rust.displayAmount(amount: transactionDetails.amount()))
            .font(.largeTitle)
            .fontWeight(.bold)
            .padding(.top, 12)

        AsyncView(operation: transactionDetails.amountFiatFmt) { amount in
            Text("≈ $\(amount) USD").foregroundStyle(.primary.opacity(0.8))
        }

        Group {
            if transactionDetails.isConfirmed() {
                TransactionCapsule(
                    text: "Sent", icon: "arrow.up.right",
                    color: .black, textColor: .white
                )
            } else {
                TransactionCapsule(
                    text: "Sending", icon: "arrow.up.right",
                    color: .coolGray, textColor: .black.opacity(0.8)
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
            SentDetailsExpandedView(manager: manager, transactionDetails: transactionDetails)
        }
    }

    @ViewBuilder
    func ScrollOrContent(content: () -> some View) -> some View {
        Group {
            if detailsExpanded {
                HStack(alignment: .top) {
                    ScrollView(.vertical) {
                        content()
                    }
                    .scrollIndicators(.never)
                    .transition(.opacity)
                    .frame(alignment: .top)
                    .scrollPosition($scrollPosition)
                }
            } else {
                VStack {
                    content()
                        .transition(.opacity)
                }
            }
        }
        .animation(.easeInOut(duration: 0.3), value: detailsExpanded)
    }

    var body: some View {
        ScrollOrContent {
            VStack(spacing: 12) {
                Spacer()
                headerIcon

                Group {
                    if transactionDetails.isReceived() {
                        ReceivedDetails
                    } else {
                        SentDetails
                    }
                }

                Spacer()
                Spacer()

                Button(action: {
                    if let url = URL(string: transactionDetails.transactionUrl()) {
                        openURL(url)
                    }
                }) {
                    Text("View in Explorer")
                        .frame(maxWidth: .infinity)
                        .padding()
                        .background(Color.midnightBtn)
                        .foregroundColor(.white)
                        .cornerRadius(8)
                        .padding(.horizontal, detailsExpandedPadding)
                }

                Button(action: {
                    if detailsExpanded {
                        withAnimation {
                            scrollPosition.scrollTo(edge: .top)
                        }
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                            manager.dispatch(action: .toggleDetailsExpanded)
                        }
                    } else {
                        manager.dispatch(action: .toggleDetailsExpanded)
                    }
                }) {
                    Text(detailsExpanded ? "Hide Details" : "Show Details")
                        .font(.footnote)
                        .fontWeight(.bold)
                        .foregroundStyle(.gray.opacity(0.8))
                        .padding(.vertical, 6)
                }
            }
        }
        .task {
            do {
                if let blockNumber = transactionDetails.blockNumber() {
                    let numberOfConfirmations = try? await manager.rust.numberOfConfirmations(
                        blockHeight: blockNumber)
                    guard numberOfConfirmations != nil else { return }
                    self.numberOfConfirmations = Int(numberOfConfirmations!)
                }
            }
        }
    }
}

#Preview("confirmed received") {
    AsyncPreview {
        TransactionDetailsView(
            id: WalletId(),
            transactionDetails: TransactionDetails.previewConfirmedReceived(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager())
    }
}

#Preview("confirmed sent") {
    AsyncPreview {
        TransactionDetailsView(
            id: WalletId(),
            transactionDetails: TransactionDetails.previewConfirmedSent(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager())
    }
}

#Preview("pending received") {
    AsyncPreview {
        TransactionDetailsView(
            id: WalletId(),
            transactionDetails: TransactionDetails.previewPendingReceived(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager())
    }
}

#Preview("pending sent") {
    AsyncPreview {
        TransactionDetailsView(
            id: WalletId(),
            transactionDetails: TransactionDetails.previewPendingSent(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager())
    }
}
