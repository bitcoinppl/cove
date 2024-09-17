//
//  TransactionDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 9/17/24.
//

import SwiftUI

struct TransactionDetailsView: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.openURL) private var openURL
    private let screenWidth = UIScreen.main.bounds.width
    private let screenHeight = UIScreen.main.bounds.height

    @State private var numberOfConfirmations: Int? = nil
//    @State private var scrollPosition = ScrollPosition()

    // public
    let id: WalletId
    let transactionDetails: TransactionDetails
    var model: WalletViewModel

    var headerIcon: HeaderIcon {
        HeaderIcon(
            isSent: transactionDetails.isSent(),
            isConfirmed: transactionDetails.isConfirmed(),
            numberOfConfirmations: numberOfConfirmations
        )
    }

    var metadata: WalletMetadata {
        model.walletMetadata
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

        Text(model.rust.displayAmount(amount: transactionDetails.amount()))
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
            ReceivedDetailsExpandedView(model: model, transactionDetails: transactionDetails, numberOfConfirmations: numberOfConfirmations)
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

        Text(model.rust.displayAmount(amount: transactionDetails.amount()))
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
            SentDetailsExpandedView(model: model, transactionDetails: transactionDetails)
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
//                    .scrollPosition(id: $scrollPosition)
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
                        .background(Color.blue)
                        .foregroundColor(.white)
                        .cornerRadius(8)
                        .padding(.horizontal, detailsExpandedPadding)
                }

                Button(action: {
                    if detailsExpanded {
                        // scroll to top, wait and then hide
                        // iOS 18
//                        scrollPosition.scrollto(.top)
//                        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
//                            model.dispatch(action: .toggleDetailsExpanded)
//                        }
                    } else {
                        model.dispatch(action: .toggleDetailsExpanded)
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
                    let numberOfConfirmations = try? await model.rust.numberOfConfirmations(blockHeight: blockNumber)
                    guard numberOfConfirmations != nil else { return }
                    self.numberOfConfirmations = Int(numberOfConfirmations!)
                }
            }
        }
    }
}

#Preview("confirmed received") {
    AsyncPreview {
        TransactionDetailsView(id: WalletId(),
                               transactionDetails: TransactionDetails.previewConfirmedReceived(),
                               model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}

#Preview("confirmed sent") {
    AsyncPreview {
        TransactionDetailsView(id: WalletId(),
                               transactionDetails: TransactionDetails.previewConfirmedSent(),
                               model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}

#Preview("pending received") {
    AsyncPreview {
        TransactionDetailsView(id: WalletId(),
                               transactionDetails: TransactionDetails.previewPendingReceived(),
                               model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}

#Preview("pending sent") {
    AsyncPreview {
        TransactionDetailsView(id: WalletId(),
                               transactionDetails: TransactionDetails.previewPendingSent(),
                               model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}
