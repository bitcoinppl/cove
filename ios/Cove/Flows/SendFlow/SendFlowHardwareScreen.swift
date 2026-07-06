//
//  SendFlowHardwareScreen.swift
//  Cove
//
//  Created by Praveen Perera on 11/21/24.
//

import Foundation
import SwiftUI

enum SendFlowHardwareSheetState: Equatable {
    case details
    case inputOutputDetails
    case exportQr
}

private enum ConfirmationState: Equatable {
    case exportTxn
    case importSignature
}

enum SendFlowHardwareAlertState: Equatable {
    case bbqrError(String)
    case fileError(String)
    case nfcError(String)
    case pasteError(String)
}

struct SendFlowHardwareScreen: View {
    @Environment(AppManager.self) private var app

    let id: WalletId
    @State var manager: WalletManager
    let details: ConfirmDetails
    let prices: PriceResponse? = nil

    // sheets, alerts, confirmations
    @State private var alertState: TaggedItem<SendFlowHardwareAlertState>? = .none
    @State private var sheetState: TaggedItem<SendFlowHardwareSheetState>? = .none
    @State private var confirmationState: TaggedItem<ConfirmationState>? = .none
    @State private var inputOutputDetailsPresentationSize: PresentationDetent = .height(300)

    /// file import
    @State private var isPresentingFilePicker = false

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var fiatAmount: String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = details.sendingAmount()
        return manager.rust.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    private var presentationContext: SendFlowHardwarePresentationContext {
        SendFlowHardwarePresentationContext(
            manager: manager,
            details: details,
            alertState: $alertState,
            inputOutputDetailsPresentationSize: $inputOutputDetailsPresentationSize
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

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
                            Text(manager.amountFmt(details.sendingAmount()))
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
                                        manager.dispatch(
                                            action: .updateUnit(.sat)
                                        )
                                    } label: {
                                        Text("sats")
                                    }

                                    Button {
                                        manager.dispatch(
                                            action: .updateUnit(.btc)
                                        )
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

                    SendFlowHardwareAccountSection(metadata: metadata)
                        .padding(.vertical)

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
                        // padding just for the context
                        .padding(16)
                        .contentShape(
                            .contextMenuPreview,
                            RoundedRectangle(cornerRadius: 8)
                        )
                        .contextMenu {
                            Button("Copy", systemImage: "doc.on.doc") {
                                UIPasteboard.general.string = details.sendingTo().unformatted()
                            }
                        }
                        // remove padding after context
                        .padding(-16)
                        .padding(.leading, 50)
                    }
                    .padding(.vertical, 8)
                    .onTapGesture { sheetState = .init(.inputOutputDetails) }

                    Divider()

                    if case let .tapSigner(ts) = metadata.hardwareMetadata {
                        SignTapSignerTransactionSection(ts)
                    } else {
                        SendFlowHardwareSignTransactionSection(
                            exportTransaction: { confirmationState = .init(.exportTxn) },
                            importSignature: { confirmationState = .init(.importSignature) }
                        )
                    }

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
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal)
            .background(Color.background)
            .presentingSheet($sheetState, context: presentationContext)
            .confirmationDialog(
                confirmationDialogTitle,
                isPresented: confirmationDialogIsPresented,
                actions: ConfirmationDialogView
            )
            .presentingAlert($alertState, context: presentationContext, defaultTitle: "Error")
            .fileImporter(
                isPresented: $isPresentingFilePicker,
                allowedContentTypes: [.plainText, .psbt, .txn],
                onCompletion: handleFileImport
            )
            .onAppear {
                let total = details.outputs().count + details.inputs().count
                if total == 3 { inputOutputDetailsPresentationSize = .height(300) }
                if total > 3 { inputOutputDetailsPresentationSize = .height(400) }
                if total > 5 { inputOutputDetailsPresentationSize = .height(500) }
            }
            .toolbar { Toolbar }
        }
    }

    @ToolbarContentBuilder
    var Toolbar: some ToolbarContent {
        ToolbarItem(placement: .navigationBarTrailing) {
            HStack {
                Button("Delete", systemImage: "trash", role: .destructive) {
                    do {
                        try manager.rust.deleteUnsignedTransaction(txId: details.id())
                        app.popRoute()
                    } catch {
                        Log.error("Unable to delete transaction \(details.id()): \(error)")
                    }
                }
                .contentShape(Rectangle())
                .tint(.white)
                .foregroundStyle(.white)
            }
        }
    }

    func handleFileImport(result: Result<URL, Error>) {
        do {
            let file = try result.get()
            let fileContents = try FileReader(for: file).read()

            let (txnRecord, parsed) = try parseSignedImport(fileContents)

            let route = parsed.sendConfirmRoute(
                id: txnRecord.walletId(),
                details: txnRecord.confirmDetails()
            )

            app.pushRoute(route)
        } catch {
            alertState = .init(.fileError(error.localizedDescription))
        }
    }

    func importPastedSignature() {
        let code = UIPasteboard.general.string ?? ""
        guard !code.isEmpty else {
            alertState = .init(.pasteError("No text found on the clipboard."))
            return
        }

        do {
            let (txnRecord, parsed) = try parseSignedImport(code)
            let route = parsed.sendConfirmRoute(
                id: txnRecord.walletId(),
                details: txnRecord.confirmDetails()
            )
            app.pushRoute(route)
        } catch {
            alertState = .init(.pasteError(error.localizedDescription))
        }
    }

    func handleScanned(_: NfcMessage?, _ txn: NfcMessage?) {
        Log.debug("handleScanned")
        guard let txn else { return }

        do {
            let parsed = try SignedTransactionOrPsbt.tryFromNfcMessage(nfcMessage: txn)
            let db = Database().unsignedTransactions()
            let txnRecord = try db.getTxThrow(txId: parsed.txId())

            let route = parsed.sendConfirmRoute(
                id: txnRecord.walletId(),
                details: txnRecord.confirmDetails()
            )

            app.pushRoute(route)
        } catch {
            Log.error("Failed to handle scanned transaction: \(error), txn: \(txn)")
            alertState = .init(.nfcError(error.localizedDescription))
        }
    }

    /// Parse signed import (PSBT or finalized transaction) and retrieve original unsigned transaction record
    func parseSignedImport(_ input: String) throws -> (UnsignedTransactionRecord, SignedTransactionOrPsbt) {
        Log.info("parseSignedImport")
        let parsed = try SignedTransactionOrPsbt.tryParse(input: input)
        let db = Database().unsignedTransactions()
        let record = try db.getTxThrow(txId: parsed.txId())
        return (record, parsed)
    }

    func SignTapSignerTransactionSection(_ ts: TapSigner) -> some View {
        VStack(spacing: 17) {
            HStack {
                Text("Sign Transaction")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundColor(.secondary)

                Spacer()
            }

            Button(action: {
                let route = TapSignerRoute.enterPin(tapSigner: ts, action: .sign(details.psbt()))
                app.sheetState = .init(.tapSigner(route))
            }) {
                Label("Sign using TAPSIGNER", systemImage: "key.card")
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 18)
                    .padding(.vertical)
                    .foregroundColor(.midnightBlue)
                    .background(.btnPrimary)
                    .cornerRadius(10)
                    .font(.caption)
                    .fontWeight(.medium)
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
        case .exportTxn:
            SendFlowHardwareExportTransactionDialog(
                exportQr: { sheetState = .init(.exportQr) },
                exportNfc: { app.nfcWriter.writeToTag(data: details.psbtBytes()) },
                shareTransaction: {
                    ShareSheet.presentFromMenu(data: details.psbtBytes(), filename: "transaction.psbt")
                }
            )
        case .importSignature:
            SendFlowHardwareImportTransactionDialog(
                scanQr: { app.sheetState = .init(.qr) },
                importFile: { isPresentingFilePicker = true },
                pasteSignature: importPastedSignature,
                scanNfc: { app.nfcReader.scan() }
            )
        case .none: EmptyView()
        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowHardwareScreen(
            id: WalletId(),
            manager: WalletManager(preview: "preview_only"),
            details: confirmDetailsPreviewNew()
        )
        .environment(AppManager.shared)
    }
}
