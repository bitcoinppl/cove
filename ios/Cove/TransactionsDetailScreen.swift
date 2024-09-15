//
//  TransactionsDetailScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/27/24.
//

import SwiftUI

struct TransactionsDetailScreen: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    // public
    let id: WalletId
    let transactionsDetails: TransactionDetails

    // private
    @State var model: WalletViewModel? = nil

    func loadModel() {
        if model != nil { return }

        do {
            Log.debug("Getting wallet model for \(id)")
            model = try app.getWalletViewModel(id: id)
        } catch {
            Log.error("Something went very wrong: \(error)")
            navigate(Route.listWallets)
        }
    }

    var body: some View {
        Group {
            if let model = model {
                TransactionDetailsView(id: id, transactionsDetails: transactionsDetails, model: model)
            } else {
                Text("Loading...")
            }
        }
        .task {
            loadModel()
        }
    }
}

struct TransactionDetailsView: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.openURL) private var openURL
    private let screenWidth = UIScreen.main.bounds.width
    private let screenHeight = UIScreen.main.bounds.height

    // public
    let id: WalletId
    let transactionsDetails: TransactionDetails
    var model: WalletViewModel

    var headerIcon: HeaderIcon {
        // pending
        if !transactionsDetails.isConfirmed() {
            return HeaderIcon(icon: "clock.arrow.2.circlepath",
                              backgroundColor: Color.coolGray,
                              checkmarkColor: .black.opacity(0.6))
        }

        // confirmed received
        if transactionsDetails.isReceived() {
            return HeaderIcon(icon: "checkmark", backgroundColor: .green, checkmarkColor: .white)
        }

        // confirmed sent
        if transactionsDetails.isSent() {
            return HeaderIcon(icon: "checkmark", backgroundColor: .black, checkmarkColor: .white)
        }

        // default
        return HeaderIcon(icon: "clock.arrow.2.circlepath", backgroundColor: .gray, checkmarkColor: .white)
    }

    var metadata: WalletMetadata {
        model.walletMetadata
    }

    var detailsExpanded: Bool {
        metadata.detailsExpanded
    }

    @ViewBuilder
    func expandedDetailsRow(header: String, content: String) -> some View {
        Text(header)
            .font(.caption)
            .foregroundColor(.gray)
            .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)

        Text(content)
            .fontWeight(.semibold)
            .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)
            .padding(.bottom, 14)
    }

    @ViewBuilder
    var ReceivedDetails: some View {
        Text(transactionsDetails.isConfirmed() ? "Transaction Received" : "Transaction Pending")
            .font(.title)
            .fontWeight(.semibold)
            .padding(.top, 8)

        // confirmed
        if transactionsDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction was successfully received on")
                    .foregroundColor(.gray)

                Text(transactionsDetails.confirmationDateTime() ?? "Unknown")
                    .fontWeight(.semibold)
                    .foregroundColor(.gray)
            }
            .multilineTextAlignment(.center)
            .padding()
        }

        // pending
        if !transactionsDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction is pending. ")
                    .foregroundColor(.gray)

                Text("Please check back soon for an update.")
                    .foregroundColor(.gray)
            }
            .multilineTextAlignment(.center)
        }

        Text(model.rust.displayAmount(amount: transactionsDetails.amount()))
            .font(.largeTitle)
            .fontWeight(.bold)
            .padding(.top, 12)

        AsyncView(operation: transactionsDetails.amountFiatFmt) { amount in
            Text("≈ $\(amount) USD").foregroundStyle(.primary.opacity(0.8))
        }

        Group {
            if transactionsDetails.isConfirmed() {
                TransactionCapsule(text: "Received", icon: "arrow.down.left", color: .green)
            } else {
                TransactionCapsule(
                    text: "Receiving", icon: "arrow.down.left",
                    color: .coolGray, textColor: .black.opacity(0.8)
                )
            }
        }
        .padding(.top, 12)

        if metadata.detailsExpanded {
            VStack(alignment: .leading) {
                Divider()
                    .padding(.vertical, 18)

                expandedDetailsRow(header: "Confirmations", content: "10")

                expandedDetailsRow(header: "Block Number", content: "840,000")

                expandedDetailsRow(header: "Received At", content: "...")
            }
        }
    }

    @ViewBuilder
    var SentDetails: some View {
        Text(transactionsDetails.isConfirmed() ? "Transaction Sent" : "Transaction Pending")
            .font(.title)
            .fontWeight(.semibold)
            .padding(.top, 6)

        // confirmed
        if transactionsDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction was sent on")
                    .foregroundColor(.gray)

                Text(transactionsDetails.confirmationDateTime() ?? "Unknown")
                    .foregroundColor(.gray)
            }
            .multilineTextAlignment(.center)
        }

        // pending
        if !transactionsDetails.isConfirmed() {
            VStack(alignment: .center, spacing: 4) {
                Text("Your transaction is pending. ")
                    .foregroundColor(.gray)

                Text("Please check back soon for an update.")
                    .fontWeight(.semibold)
                    .foregroundColor(.gray)
            }
            .multilineTextAlignment(.center)
        }

        Text(model.rust.displayAmount(amount: transactionsDetails.amount()))
            .font(.largeTitle)
            .fontWeight(.bold)
            .padding(.top, 12)

        AsyncView(operation: transactionsDetails.amountFiatFmt) { amount in
            Text("≈ $\(amount) USD").foregroundStyle(.primary.opacity(0.8))
        }

        Group {
            if transactionsDetails.isConfirmed() {
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
    }

    @ViewBuilder
    func ScrollOrContent(content: () -> some View) -> some View {
        if detailsExpanded {
            ScrollView(.vertical) {
                content()
            }
            .scrollIndicators(.never)
        } else {
            content()
        }
    }

    var body: some View {
        ScrollOrContent {
            VStack(spacing: 12) {
                headerIcon

                Group {
                    if transactionsDetails.isReceived() {
                        ReceivedDetails
                    } else {
                        SentDetails
                    }
                }
                .padding(.horizontal, 28)

                Spacer()
                Spacer()

                Button(action: {
                    if let url = URL(string: transactionsDetails.transactionUrl()) {
                        openURL(url)
                    }
                }) {
                    Text("View in Explorer")
                        .frame(maxWidth: .infinity)
                        .padding()
                        .background(Color.blue)
                        .foregroundColor(.white)
                        .cornerRadius(8)
                        .padding(.horizontal, 12)
                }

                Button(action: {
                    model.dispatch(action: .toggleDetailsExpanded)
                }) {
                    Text(detailsExpanded ? "Hide Details" : "Show Details")
                        .font(.footnote)
                        .fontWeight(.bold)
                        .foregroundStyle(.gray.opacity(0.8))
                        .padding(.vertical, 6)
                }
            }
            .padding(.horizontal, 24)
        }
    }
}

struct HeaderIcon: View {
    // passed in
    var icon: String = "checkmark"
    var backgroundColor: Color = .green
    var checkmarkColor: Color = .white
    var ringColor: Color? = nil

    // private
    private let screenWidth = UIScreen.main.bounds.width
    private var circleSize: CGFloat {
        screenWidth * 0.33
    }

    private func circleOffSet(of offset: CGFloat) -> CGFloat {
        circleSize + (offset * 20)
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(backgroundColor)
                .frame(width: circleSize, height: circleSize)

            Circle()
                .stroke(ringColor ?? backgroundColor, lineWidth: 1)
                .frame(width: circleOffSet(of: 1), height: circleOffSet(of: 1))
                .opacity(0.44)

            Circle()
                .stroke(ringColor ?? backgroundColor, lineWidth: 1)
                .frame(width: circleOffSet(of: 2), height: circleOffSet(of: 2))
                .opacity(0.24)

            Circle()
                .stroke(ringColor ?? backgroundColor, lineWidth: 1)
                .frame(width: circleOffSet(of: 3), height: circleOffSet(of: 3))
                .opacity(0.06)

            Image(systemName: icon)
                .foregroundColor(checkmarkColor)
                .font(.system(size: 62))
        }
    }
}

#Preview("confirmed received") {
    AsyncPreview {
        TransactionDetailsView(id: WalletId(),
                               transactionsDetails: TransactionDetails.previewConfirmedReceived(),
                               model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}

#Preview("confirmed sent") {
    AsyncPreview {
        TransactionDetailsView(id: WalletId(),
                               transactionsDetails: TransactionDetails.previewConfirmedSent(),
                               model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}

#Preview("pending received") {
    AsyncPreview {
        TransactionDetailsView(id: WalletId(),
                               transactionsDetails: TransactionDetails.previewPendingReceived(),
                               model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}

#Preview("pending sent") {
    AsyncPreview {
        TransactionDetailsView(id: WalletId(),
                               transactionsDetails: TransactionDetails.previewPendingSent(),
                               model: WalletViewModel(preview: "preview_only"))
            .environment(MainViewModel())
    }
}
