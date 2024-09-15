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

    // public
    let id: WalletId
    let transactionsDetails: TransactionDetails
    var model: WalletViewModel

    var headerIcon: HeaderIcon {
        if transactionsDetails.isConfirmed() {
            return HeaderIcon(icon: "checkmark", backgroundColor: .green, checkmarkColor: .white)
        } else {
            return HeaderIcon(icon: "clock.arrow.2.circlepath", backgroundColor: .gray, checkmarkColor: .white)
        }
    }

    @ViewBuilder
    var ReceivedDetails: some View {
        Text("Transfer Received")
            .font(.title)
            .fontWeight(.semibold)

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

        Text("≈ $100 USD").foregroundStyle(.primary.opacity(0.8))

        TransactionCapsule(text: "Received", icon: "arrow.up.right", color: .green)
            .padding(.top, 12)
    }

    var body: some View {
        VStack(spacing: 12) {
            Spacer()
            headerIcon

            if transactionsDetails.isReceived() {
                ReceivedDetails
            }

            Spacer()
            Spacer()

            Button(action: {
                // Action to perform when button is tapped
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
                // Action to perform when button is tapped
            }) {
                Text("More Details")
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
