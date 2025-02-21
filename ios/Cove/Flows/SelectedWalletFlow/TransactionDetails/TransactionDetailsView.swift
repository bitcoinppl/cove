//
//  TransactionDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct TransactionDetailsView: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(AppManager.self) private var app
    @Environment(\.openURL) private var openURL

    private let screenWidth = UIScreen.main.bounds.width
    private let screenHeight = UIScreen.main.bounds.height

    @State private var numberOfConfirmations: Int? = nil
    @State private var scrollPosition = ScrollPosition()

    @State private var initialOffset: Double? = nil
    @State private var currentOffset: Double = 0

    // public
    let id: WalletId
    @State var transactionDetails: TransactionDetails
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
        VStack {
            headerIcon

            VStack(spacing: 4) {
                Text(transactionDetails.isConfirmed() ? "Transaction Received" : "Transaction Pending")
                    .font(.title)
                    .fontWeight(.semibold)
                    .padding(.top, 8)

                // add, edit, remove label
                TransactionDetailsLabelView(details: transactionDetails, manager: manager)
            }
        }

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

        VStack(spacing: 8) {
            Text(manager.rust.displayAmount(amount: transactionDetails.amount()))
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top, 12)

            AsyncView(operation: transactionDetails.amountFiatFmt) { amount in
                Text(amount).foregroundStyle(.primary.opacity(0.8))
            }
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
        VStack {
            headerIcon

            VStack(spacing: 4) {
                Text(transactionDetails.isConfirmed() ? "Transaction Sent" : "Transaction Pending")
                    .font(.title)
                    .fontWeight(.semibold)
                    .padding(.top, 6)

                // add, edit, remove label
                TransactionDetailsLabelView(details: transactionDetails, manager: manager)
            }
        }

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

        VStack(spacing: 8) {
            Text(manager.rust.displayAmount(amount: transactionDetails.amount()))
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top, 12)

            AsyncView(operation: transactionDetails.amountFiatFmt) { amount in
                Text(amount).foregroundStyle(.primary.opacity(0.8))
            }
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
    func ContentScrollView(content: () -> some View) -> some View {
        ScrollView(.vertical) {
            content()
        }
        .scrollIndicators(.never)
        .frame(alignment: .top)
        .scrollPosition($scrollPosition)
        .scrollDisabled(!detailsExpanded)
        .onScrollGeometryChange(for: Double.self) { geo in
            geo.contentOffset.y
        } action: { oldValue, newValue in
            if oldValue == newValue { return }
            if oldValue == 0 { return }
            let initialOffset = initialOffset ?? oldValue
            self.initialOffset = initialOffset
            currentOffset = initialOffset - newValue
        }
    }

    var body: some View {
        ContentScrollView {
            VStack(spacing: 24) {
                Spacer()
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
                        .cornerRadius(10)
                        .padding(.horizontal, detailsExpandedPadding)
                }

                Button(action: {
                    if detailsExpanded {
                        withAnimation { scrollPosition.scrollTo(edge: .top) }
                        manager.dispatch(action: .toggleDetailsExpanded)
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
        .background(
            GeometryReader { geometry in
                Image(.transactionDetailsPattern)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: geometry.size.width, alignment: .center)
                    .ignoresSafeArea(edges: .top)
                    .opacity(colorScheme == .light ? 0.40 : 1)
                    .offset(y: currentOffset > 0 ? 0 : currentOffset)
                    .opacity(max(0, 1 + (currentOffset / 275)))
            }
        )
    }

    var backgroundImageOffset: CGFloat {
        guard detailsExpanded else { return 0 }
        guard currentOffset < 0 else { return 0 }
        return currentOffset
    }
}

#Preview("confirmed received") {
    AsyncPreview {
        TransactionDetailsView(
            id: WalletId(),
            transactionDetails: TransactionDetails.previewConfirmedReceived(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("confirmed sent") {
    AsyncPreview {
        TransactionDetailsView(
            id: WalletId(),
            transactionDetails: TransactionDetails.previewConfirmedSent(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("pending received") {
    AsyncPreview {
        TransactionDetailsView(
            id: WalletId(),
            transactionDetails: TransactionDetails.previewPendingReceived(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}

#Preview("pending sent") {
    AsyncPreview {
        TransactionDetailsView(
            id: WalletId(),
            transactionDetails: TransactionDetails.previewPendingSent(),
            manager: WalletManager(preview: "preview_only")
        )
        .environment(AppManager.shared)
    }
}
