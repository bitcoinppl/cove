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

    private let detailsExpandedPadding: CGFloat = 28

    // state
    @State private var isCopied = false
    @State private var numberOfConfirmations: Int? = nil

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
                Text("Your transaction was successfully received")
                    .foregroundColor(.gray)

                Text(transactionsDetails.confirmationDateTime() ?? "Unknown")
                    .fontWeight(.semibold)
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
            VStack(alignment: .leading) {
                Divider().padding(.vertical, 18)

                if transactionsDetails.isConfirmed() {
                    Text("Confirmations")
                        .font(.caption)
                        .foregroundColor(.gray)
                        .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)

                    if let numberOfConfirmations = self.numberOfConfirmations {
                        Text(ThousandsFormatter(numberOfConfirmations).fmt())
                            .fontWeight(.semibold)
                            .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)
                            .padding(.bottom, 14)
                    } else {
                        ProgressView()
                    }

                    expandedDetailsRow(header: "Block Number", content: String(transactionsDetails.blockNumberFmt() ?? ""))
                }

                Text("Received At")
                    .font(.caption)
                    .foregroundColor(.gray)
                    .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)

                HStack {
                    Text(transactionsDetails.addressSpacedOut())
                        .fontWeight(.semibold)
                        .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)
                        .padding(.bottom, 14)

                    Spacer()
                    Spacer()

                    Button(action: {
                        UIPasteboard.general.string = transactionsDetails.address().string()
                        withAnimation {
                            isCopied = true
                        }

                        // Reset the button text after a delay
                        DispatchQueue.main.asyncAfter(deadline: .now() + 5) {
                            withAnimation {
                                isCopied = false
                            }
                        }
                    }) {
                        HStack(spacing: 8) {
                            Image(systemName: "doc.on.doc")
                                .font(.caption)

                            Text(isCopied ? "Copied" : "Copy")
                                .font(.caption)
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(Color.white)
                        .foregroundColor(.black)
                        .overlay(
                            RoundedRectangle(cornerRadius: 20)
                                .stroke(Color.gray.opacity(0.3), lineWidth: 1)
                        )
                    }
                    .buttonStyle(PlainButtonStyle())
                }
            }
            .padding(.horizontal, detailsExpandedPadding)
            .task {
                do {
                    if let blockNumber = transactionsDetails.blockNumber() {
                        let numberOfConfirmations = try? await model.rust.numberOfConfirmations(blockHeight: blockNumber)
                        guard numberOfConfirmations != nil else { return }
                        self.numberOfConfirmations = Int(numberOfConfirmations!)
                    }
                }
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

        if let confirmations = numberOfConfirmations, confirmations < 3 {
            VStack {
                Divider().padding(.vertical, 18)
                ConfirmationIndicatorView(current: confirmations)
            }
            .padding(.horizontal, detailsExpandedPadding)
        }

        // MARK: Details Expanded

        if metadata.detailsExpanded {
            VStack(alignment: .leading, spacing: 12) {
                Divider().padding(.vertical, 18)

                VStack(alignment: .leading, spacing: 8) {
                    Text("Sent to")
                        .font(.footnote)
                        .foregroundColor(.secondary)
                        .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)

                    Text(transactionsDetails.addressSpacedOut())
                        .fontWeight(.semibold)
                        .multilineTextAlignment(/*@START_MENU_TOKEN@*/ .leading/*@END_MENU_TOKEN@*/)
                        .textSelection(.enabled)

                    if transactionsDetails.isConfirmed() {
                        HStack(spacing: 0) {
                            Group {
                                Text(transactionsDetails.blockNumberFmt() ?? "")
                                Text("|")

                                AsyncView(operation: {
                                    let blockNumber = transactionsDetails.blockNumber() ?? 0
                                    return try await model.rust.numberOfConfirmationsFmt(blockHeight: blockNumber)
                                }) { (confirmations: String) in
                                    Group {
                                        Text(confirmations)

                                        Image(systemName: "checkmark.circle.fill")
                                            .font(.system(size: 10))
                                            .fontWeight(.bold)
                                            .foregroundStyle(.green)
                                            .padding(.leading, 3)
                                    }
                                }
                            }

                            .padding(.horizontal, 2)
                        }
                        .font(.caption).foregroundStyle(.tertiary)
                    }
                }

                Divider().padding(.vertical, 18)

                HStack {
                    Text("Network Fee")
                    Image(systemName: "info.circle")
                        .font(.footnote)
                        .fontWeight(/*@START_MENU_TOKEN@*/ .bold/*@END_MENU_TOKEN@*/)
                        .foregroundStyle(.tertiary.opacity(0.8))
                    Spacer()
                    Text(transactionsDetails.feeFmt(unit: metadata.selectedUnit) ?? "")
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)

                HStack {
                    Text("Receipient Receives")
                    Spacer()
                    Text(transactionsDetails.sentSansFeeFmt(unit: metadata.selectedUnit) ?? "")
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)

                Divider().padding(.vertical, 18)

                HStack(alignment: .top) {
                    Text("Total Spent")

                    Spacer()
                    VStack(alignment: .trailing) {
                        Text(transactionsDetails.amountFmt(unit: metadata.selectedUnit))
                        AsyncView(operation: transactionsDetails.amountFiatFmt) { amount in
                            Text("≈ $\(amount) USD").foregroundStyle(.secondary)
                                .font(.caption)
                                .padding(.top, 2)
                        }
                    }
                }
                .font(.subheadline)
            }
            .padding(.horizontal, detailsExpandedPadding)
        }
    }

    @ViewBuilder
    func ScrollOrContent(content: () -> some View) -> some View {
        Group {
            if detailsExpanded {
                ScrollView(.vertical) {
                    content()
                }
                .scrollIndicators(.never)
                .transition(.opacity)
            } else {
                content()
                    .transition(.opacity)
            }
        }
        .animation(.easeInOut(duration: 0.3), value: detailsExpanded)
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
                        .padding(.horizontal, detailsExpandedPadding)
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
