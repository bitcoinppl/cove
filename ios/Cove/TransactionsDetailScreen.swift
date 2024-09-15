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

    // public
    let id: WalletId
    let transactionsDetails: TransactionDetails
    var model: WalletViewModel

    var headerIcon: HeaderIcon {
        // pending
        if !transactionsDetails.isConfirmed() {
            return HeaderIcon(icon: "clock.arrow.2.circlepath", backgroundColor: .gray, checkmarkColor: .white)
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
    var ReceivedDetails: some View {
        Text("Transaction Received")
            .font(.title)
            .fontWeight(.semibold)
            .padding(.top, 8)

        VStack(alignment: .center, spacing: 4) {
            Text("Your transaction was successfully received on")
                .foregroundColor(.gray)

            Text(transactionsDetails.confirmationDateTime() ?? "Unknown")
                .fontWeight(.semibold)
                .foregroundColor(.gray)
        }
        .multilineTextAlignment(.center)
        .padding()

        Text(model.rust.displayAmount(amount: transactionsDetails.amount()))
            .font(.largeTitle)
            .fontWeight(.bold)
            .padding(.top, 6)

        AsyncView(operation: transactionsDetails.amountFiatFmt) { amount in
            Text("≈ $\(amount) USD").foregroundStyle(.primary.opacity(0.8))
        }

        TransactionCapsule(text: "Received", icon: "arrow.down.left", color: .green)
            .padding(.top, 12)
    }

    @ViewBuilder
    var SentDetails: some View {
        Text("Transaction Sent")
            .font(.title)
            .fontWeight(.semibold)
            .padding(.top, 8)

        VStack(alignment: .center, spacing: 4) {
            Text("Your transaction was sent on")
                .foregroundColor(.gray)

            Text(transactionsDetails.confirmationDateTime() ?? "Unknown")
                .fontWeight(.semibold)
                .foregroundColor(.gray)
        }
        .multilineTextAlignment(.center)
        .padding()

        Text(model.rust.displayAmount(amount: transactionsDetails.amount()))
            .font(.largeTitle)
            .fontWeight(.bold)
            .padding(.top, 6)

        AsyncView(operation: transactionsDetails.amountFiatFmt) { amount in
            Text("≈ $\(amount) USD").foregroundStyle(.primary.opacity(0.8))
        }

        TransactionCapsule(text: "Sent", icon: "arrow.up.right", color: .black, textColor: .white)
            .padding(.top, 12)
    }

    var body: some View {
        VStack(spacing: 12) {
            Spacer()
            headerIcon

            if transactionsDetails.isReceived() {
                ReceivedDetails
            } else {
                SentDetails
            }

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
                    .padding(.horizontal, 16)
            }
            .padding(.horizontal)

            Button(action: {
                model.dispatch(action: .toggleDetailsExpanded)
            }) {
                Text(detailsExpanded ? "Hide Details" : "Show Details")
                    .font(.footnote)
                    .fontWeight(.bold)
                    .foregroundStyle(.gray.opacity(0.8))
                    .padding(.vertical, 6)
            }
            .padding(.horizontal)
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
                .font(.system(size: 50))
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
