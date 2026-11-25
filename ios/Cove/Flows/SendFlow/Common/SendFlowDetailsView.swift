//
//  SendFlowDetailsView.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import SwiftUI

struct SendFlowDetailsView: View {
    @Environment(AppManager.self) private var app
    @Environment(SendFlowPresenter.self) private var presenter

    // args
    let manager: WalletManager
    let details: ConfirmDetails
    @State var prices: PriceResponse?

    /// The picked wallet the send pays, shown in place of the bare address
    var recipient: TransferRecipient? = nil

    // private
    @State private var presentingInputOutputDetails = false
    @State private var presentationSize: PresentationDetent = .medium

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    func fiatAmount(_ amount: Amount) -> String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        return manager.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    func displayFiatOrBtcAmount(_ amount: Amount) -> String {
        switch metadata.fiatOrBtc {
        case .fiat:
            return "≈ \(fiatAmount(amount))"
        case .btc:
            let units = metadata.selectedUnit == .sat ? "sats" : "btc"
            return "\(manager.amountFmt(amount)) \(units)"
        }
    }

    var body: some View {
        VStack(spacing: 12) {
            Group {
                if let recipient {
                    SendFlowDetailsRecipientCard(recipient: recipient)
                } else {
                    SendFlowDetailsAddressRow(address: details.sendingTo())
                }
            }
            .padding(.top, 6)
            .onTapGesture { presentingInputOutputDetails = true }

            SendFlowDetailsAmountRows(
                feeAmount: displayFiatOrBtcAmount(details.feeTotal()),
                feePercentage: details.feePercentage(),
                receiveAmount: displayFiatOrBtcAmount(details.sendingAmount()),
                spendingAmount: displayFiatOrBtcAmount(details.spendingAmount())
            )
            .onTapGesture { manager.dispatch(action: .toggleFiatOrBtc) }
        }
        .onChange(of: app.prices, initial: true) { _, newPrices in
            guard let prices = newPrices else { return }
            self.prices = prices
        }
        .sheet(isPresented: $presentingInputOutputDetails) {
            SendFlowAdvancedDetailsView(manager: manager, details: details)
                .presentationDetents(
                    [.height(300), .height(400), .height(500), .large], selection: $presentationSize
                )
        }
        .onAppear(perform: updatePresentationSize)
    }

    private func updatePresentationSize() {
        let total = details.outputs().count + details.inputs().count
        if total == 3 { presentationSize = .height(300) }
        if total > 3 { presentationSize = .height(400) }
        if total > 5 { presentationSize = .height(500) }
    }
}

/// The user's own wallet a transfer pays, with the address it handed out
private struct SendFlowDetailsRecipientCard: View {
    let recipient: TransferRecipient

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("TO WALLET")
                .font(.caption2)
                .fontWeight(.semibold)
                .tracking(0.6)
                .foregroundStyle(.secondary)

            HStack(spacing: 8) {
                Circle()
                    .fill(Color(recipient.color))
                    .frame(width: 8, height: 8)

                Text(recipient.name)
                    .font(.subheadline)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                    .accessibilityIdentifier("sendFlowConfirm.transferRecipient.name")
            }

            Divider()

            // no line limit: the card asks the user to check this address, so it shows all of it
            Text(address: recipient.address)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            // Cove derived a hardware wallet's address from its public key, so the device itself
            // is the one place that can confirm it
            if recipient.addressCheck == .onDevice {
                Text("Check this address on your hardware wallet before you send.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("sendFlowConfirm.transferRecipient.checkOnDevice")
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 16).fill(Color.systemGray6))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("sendFlowConfirm.transferRecipient")
    }
}

private struct SendFlowDetailsAddressRow: View {
    let address: Address

    var body: some View {
        HStack {
            Text("Address")
                .font(.footnote)
                .fontWeight(.medium)
                .foregroundStyle(.secondary)
                .foregroundColor(.primary)

            Spacer()
            Spacer()
            Spacer()
            Spacer()

            Text(address: address)
                .font(.footnote)
                .fontWeight(.semibold)
                .padding(.leading, 60)
                .lineLimit(3)
        }
    }
}

private struct SendFlowDetailsAmountRows: View {
    let feeAmount: String
    let feePercentage: UInt64
    let receiveAmount: String
    let spendingAmount: String

    var body: some View {
        HStack {
            Text("Network Fee")
                .font(.footnote)
                .fontWeight(.medium)
                .foregroundStyle(.secondary)

            Spacer()

            Text(feeAmount)
                .font(.footnote)
                .fontWeight(feePercentage > 20 ? .bold : .medium)
                .foregroundStyle(feePercentage > 20 ? .red : .secondary)
                .padding(.vertical, 10)
                .accessibilityIdentifier("sendFlowConfirm.fee")
        }

        HStack {
            Text("They'll receive")
            Spacer()
            Text(receiveAmount)
                .accessibilityIdentifier("sendFlowConfirm.receive")
        }
        .font(.footnote)
        .fontWeight(.semibold)

        HStack {
            Text("You'll pay")
            Spacer()
            Text(spendingAmount)
                .accessibilityIdentifier("sendFlowConfirm.total")
        }
        .font(.footnote)
        .fontWeight(.semibold)
    }
}

#Preview {
    AsyncPreview {
        SendFlowDetailsView(
            manager: WalletManager(preview: .only),
            details: confirmDetailsPreviewNew(),
            prices: nil
        )
        .padding()
        .environment(AppManager.shared)
        .environment(
            SendFlowPresenter(
                app: AppManager.shared, manager: WalletManager(preview: .only)
            )
        )
    }
}
