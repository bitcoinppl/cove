//
//  SendFlowHardwareScreen.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import Foundation
import SwiftUI

private enum SheetState: Equatable {
    case details
    case exportQr([String])
}

private enum ConfirmationState: Equatable {
    case exportTxn
    case importSignature
}

struct SendFlowHardwareScreen: View {
    @Environment(MainViewModel.self) private var app

    let id: WalletId
    @State var model: WalletViewModel
    let details: ConfirmDetails
    let prices: PriceResponse? = nil

    // private
    let nfcWriter = NFCWriter()
    @State private var sheetState: TaggedItem<SheetState>? = .none
    @State private var confirmationState: TaggedItem<ConfirmationState>? = .none

    var metadata: WalletMetadata {
        model.walletMetadata
    }

    var fiatAmount: String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = details.sendingAmount().asBtc() * Double(prices.usd)
        return model.fiatAmountToString(amount)
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(model: model, amount: model.balance.confirmed)

            // MARK: CONTENT

            ScrollView {
                VStack(spacing: 24) {
                    // amount
                    VStack(spacing: 8) {
                        HStack {
                            Text("You're sending")
                                .font(.headline)
                                .fontWeight(.bold)

                            Spacer()
                        }
                        .padding(.top, 6)

                        HStack {
                            Text("The amount they will receive")
                                .font(.footnote)
                                .foregroundStyle(.secondary.opacity(0.80))
                                .fontWeight(.medium)
                            Spacer()
                        }
                    }
                    .padding(.top)

                    // Balance Section
                    VStack(spacing: 8) {
                        HStack(alignment: .bottom) {
                            Text(model.amountFmt(details.sendingAmount()))
                                .font(.system(size: 48, weight: .bold))
                                .minimumScaleFactor(0.01)
                                .lineLimit(1)

                            Text(metadata.selectedUnit == .sat ? "sats" : "btc")
                                .padding(.vertical, 10)
                                .padding(.horizontal, 16)
                                .contentShape(
                                    .contextMenuPreview,
                                    RoundedRectangle(cornerRadius: 8)
                                )
                                .contextMenu {
                                    Button {
                                        model.dispatch(
                                            action: .updateUnit(.sat))
                                    } label: {
                                        Text("sats")
                                    }

                                    Button {
                                        model.dispatch(
                                            action: .updateUnit(.btc))
                                    } label: {
                                        Text("btc")
                                    }
                                } preview: {
                                    Text(
                                        metadata.selectedUnit == .sat
                                            ? "sats" : "btc"
                                    )
                                    .padding(.vertical, 10)
                                    .padding(.horizontal)
                                }
                                .offset(y: -5)
                                .offset(x: -16)
                        }
                        .offset(x: 32)

                        Text(fiatAmount)
                            .font(.title3)
                            .foregroundColor(.secondary)
                    }
                    .padding(.top, 8)

                    AccountSection.padding(.vertical)

                    Divider()

                    // MARK: To Address Section

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

                        Text(
                            details.sendingTo().spacedOut()
                        )
                        .lineLimit(4, reservesSpace: false)
                        .font(.system(.footnote, design: .none))
                        .fontWeight(.semibold)
                        .padding(.leading, 60)
                    }
                    .padding(.vertical, 8)

                    Divider()

                    // sign Transaction Section
                    SignTransactionSection

                    Spacer()

                    // more details button
                    Button(action: { sheetState = .init(.details) }) {
                        Text("More details")
                    }
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fontWeight(.medium)
                }
            }
            .scrollIndicators(.hidden)
            .background(Color.coveBg)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal)
            .sheet(item: $sheetState, content: SheetContent)
            .confirmationDialog(
                confirmationDialogTitle, isPresented: confirmationDialogIsPresented,
                actions: ConfirmationDialogView
            )
        }
    }

    @ViewBuilder
    var AccountSection: some View {
        VStack {
            HStack {
                BitcoinShieldIcon(width: 24, color: .orange)

                VStack(alignment: .leading, spacing: 6) {
                    Text(
                        metadata.masterFingerprint?.asUppercase()
                            ?? "No Fingerprint"
                    )
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundColor(.secondary)

                    Text(metadata.name)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
                .padding(.leading, 8)

                Spacer()
            }
        }
    }

    @ViewBuilder
    var SignTransactionSection: some View {
        VStack(spacing: 17) {
            HStack {
                Text("Sign Transaction")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundColor(.secondary)

                Spacer()
            }

            HStack {
                Button(action: {
                    confirmationState = .init(.exportTxn)
                }) {
                    Label("Export Transaction", systemImage: "square.and.arrow.up")
                        .padding(.horizontal, 18)
                        .padding(.vertical)
                        .foregroundColor(.midnightBlue)
                        .background(.buttonPrimary)
                        .cornerRadius(8)
                        .font(.caption)
                        .fontWeight(.medium)
                }

                Spacer()

                Button(action: {
                    confirmationState = .init(.importSignature)
                }) {
                    Label("Import Signature", systemImage: "square.and.arrow.down")
                        .padding(.horizontal, 18)
                        .padding(.vertical)
                        .foregroundColor(.midnightBlue)
                        .background(.buttonPrimary)
                        .cornerRadius(8)
                        .font(.caption)
                        .fontWeight(.medium)
                }
            }
        }
    }

    // MARK: Confirmation Dialog

    var confirmationDialogIsPresented: Binding<Bool> {
        Binding(
            get: { confirmationState != .none },
            set: { presented in
                if !presented { confirmationState = .none }
            }
        )
    }

    var confirmationDialogTitle: Text {
        switch confirmationState?.item {
        case .exportTxn: Text("Export Transaction")
        case .importSignature: Text("Import Signature")
        case .none: Text("")
        }
    }

    @ViewBuilder
    func ConfirmationDialogView() -> some View {
        switch confirmationState?.item {
        case .exportTxn: ExportTransactionDialog
        case .importSignature: ImportTransactionDialog
        case .none: EmptyView()
        }
    }

    @ViewBuilder
    var ExportTransactionDialog: some View {
        Button("QR Code") {
            do {
                let qrs = try details.psbtToBbqr()
                sheetState = .init(.exportQr(qrs))
            } catch {
                print("Failed to convert PSBT to BBQR: \(error)")
                // TODO: show alert
            }
        }

        Button("NFC") {
            nfcWriter.writeToTag(data: details.psbtBytes())
        }

        ShareLink(
            item: PSBTFile(data: details.psbtBytes(), filename: "transaction.psbt"),
            preview: SharePreview(
                "transaction.psbt - A Partially Signed Bitcoin Transaction",
                image: Image(.bitcoinShield)
            )
        )
    }

    @ViewBuilder
    var ImportTransactionDialog: some View {
        Text("TODO")
    }

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .details:
            SendFlowDetailsSheetView(model: model, details: details)
                .presentationDetents([.height(425), .height(600), .large])
                .padding()
        case let .exportQr(qrs):
            SendFlowBbqrExport(qrs: qrs.map { QrCodeView(text: $0) })
                .presentationDetents([.height(425), .height(600), .large])
                .padding()
        }
    }
}

#Preview {
    NavigationStack {
        AsyncPreview {
            SendFlowHardwareScreen(
                id: WalletId(),
                model: WalletViewModel(preview: "preview_only"),
                details: ConfirmDetails.previewNew()
            )
            .environment(MainViewModel())
        }
    }
}
