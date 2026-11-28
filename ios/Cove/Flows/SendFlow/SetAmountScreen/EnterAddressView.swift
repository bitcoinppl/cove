//
//  EnterAddressView.swift
//  Cove
//
//  Created by Praveen Perera on 11/19/24.
//
import SwiftUI

// MARK: Aliases

private typealias FocusField = SendFlowPresenter.FocusField

/// The send's "Send to" section: the address card, or the picked wallet's card, and the row that
/// opens the user's other wallets
struct EnterAddressView: View {
    @Environment(AppManager.self) private var app
    @Environment(SendFlowPresenter.self) private var presenter
    @Environment(SendFlowManager.self) private var sendFlowManager

    /// args
    @Binding var address: String

    /// The first few other wallets' colors, for the dots on the row that opens them
    ///
    /// app.wallets holds only this network and mode, so any other wallet is one to send to
    private var otherWalletColors: [WalletColor] {
        app.wallets
            .filter { $0.id != sendFlowManager.id }
            .prefix(3)
            .map(\.color)
    }

    /// The picked wallet's card shows until the user goes back to editing the address
    private var shownRecipient: TransferRecipient? {
        guard presenter.focusField != .address else { return nil }
        return sendFlowManager.transferRecipient
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Send to")
                .font(.headline)
                .fontWeight(.bold)
                .id(FocusField.address)

            if let recipient = shownRecipient {
                TransferRecipientCard(
                    recipient: recipient,
                    change: showWalletPicker,
                    edit: focusAddress
                )
            } else {
                AddressCard(address: $address, scan: showQrScanner)
            }

            if !otherWalletColors.isEmpty {
                SendBetweenWalletsRow(colors: otherWalletColors, open: showWalletPicker)
            }
        }
        .onChange(of: address, initial: true) { _, new in
            let noSpaces = new.replacingOccurrences(of: " ", with: "").trimmingCharacters(
                in: .whitespaces
            )

            if noSpaces != new { address = noSpaces }
        }
        .padding(.vertical, 14)
    }

    private func focusAddress() {
        presenter.focusField = .address
    }

    private func showQrScanner() {
        presenter.sheetState = TaggedItem(.qr)
    }

    private func showWalletPicker() {
        presenter.sheetState = TaggedItem(.transferDestination)
    }
}

/// The address field on a rounded card, with the scan button inside it
private struct AddressCard: View {
    @Environment(SendFlowPresenter.self) private var presenter

    @Binding var address: String
    let scan: () -> Void

    /// private
    @FocusState private var focusField: SendFlowPresenter.FocusField?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            AddressTextEditor(text: $address)
                .focused($focusField, equals: .address)
                .foregroundStyle(.primary.opacity(0.9))
                .autocorrectionDisabled(true)
                .keyboardType(.asciiCapable)

            Button(action: scan) {
                Image(systemName: "qrcode.viewfinder")
                    .font(.title3)
            }
            .foregroundStyle(.secondary)
            .padding(.top, 6)
            .accessibilityLabel("Scan address")
            .accessibilityIdentifier("sendFlow.scanAddress")
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color.systemGray6))
        .contentShape(Rectangle())
        .onTapGesture { presenter.focusField = .address }
        .onChange(of: presenter.focusField, initial: true) { _, new in focusField = new }
        .onChange(of: focusField, initial: false) { _, new in
            guard let new else { return }
            presenter.focusField = new
        }
    }
}

/// The wallet the user picked, with the address it handed out; tapping the address goes back to
/// editing it
private struct TransferRecipientCard: View {
    let recipient: TransferRecipient
    let change: () -> Void
    let edit: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Circle()
                    .fill(Color(recipient.color))
                    .frame(width: 8, height: 8)

                Text(recipient.name)
                    .font(.subheadline)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                    .accessibilityIdentifier("sendFlow.transferRecipient.name")

                Spacer()

                Button("Change", action: change)
                    .font(.subheadline)
                    .accessibilityIdentifier("sendFlow.transferRecipient.change")
            }

            Text(address: recipient.address)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .padding(.leading, 16)
                .accessibilityIdentifier("sendFlow.transferRecipient.address")
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color.systemGray6))
        .contentShape(Rectangle())
        .onTapGesture(perform: edit)
    }
}

/// Opens the user's other wallets, shown whenever there is one to send to
private struct SendBetweenWalletsRow: View {
    let colors: [WalletColor]
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 10) {
                StackedWalletDots(colors: colors)

                Text("Send between wallets")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundStyle(.primary)

                Spacer()

                Image(systemName: "chevron.right")
                    .font(.footnote)
                    .fontWeight(.semibold)
                    .foregroundStyle(.tertiary)
            }
            .padding(.vertical, 6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("sendFlow.sendBetweenWallets")
    }
}

/// Overlapping dots in the other wallets' colors, each ringed so the overlap reads
private struct StackedWalletDots: View {
    let colors: [WalletColor]

    var body: some View {
        HStack(spacing: -6) {
            ForEach(Array(colors.enumerated()), id: \.offset) { _, color in
                Circle()
                    .fill(Color(color))
                    .frame(width: 16, height: 16)
                    .overlay(Circle().stroke(Color(.systemBackground), lineWidth: 1.5))
            }
        }
        .accessibilityHidden(true)
    }
}
