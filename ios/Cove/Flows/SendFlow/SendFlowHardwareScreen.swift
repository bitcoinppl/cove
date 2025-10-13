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
    case inputOutputDetails
    case exportQr([String])
}

private enum DetailsSheetState: Equatable {
    case main
    case inputOutputDetails
}

private enum ConfirmationState: Equatable {
    case exportTxn
    case importSignature
}

private enum AlertState: Equatable {
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
    @State private var alertState: TaggedItem<AlertState>? = .none
    @State private var sheetState: TaggedItem<SheetState>? = .none
    @State private var confirmationState: TaggedItem<ConfirmationState>? = .none
    @State private var inputOutputDetailsPresentationSize: PresentationDetent = .height(300)

    // file import
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
                                            action: .updateUnit(.sat))
                                    } label: {
                                        Text("sats")
                                    }

                                    Button {
                                        manager.dispatch(
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

                    AccountSection
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
                        SignTransactionSection
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
            .sheet(item: $sheetState, content: SheetContent)
            .alert(
                alertTitle,
                isPresented: showingAlert,
                presenting: alertState,
                actions: { MyAlert($0).actions },
                message: { MyAlert($0).message }
            )
            .confirmationDialog(
                confirmationDialogTitle,
                isPresented: confirmationDialogIsPresented,
                actions: ConfirmationDialogView
            )
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

            let (txnRecord, signedTransaction) = try txnRecordAndSignedTxn(fileContents)

            let route = RouteFactory()
                .sendConfirm(
                    id: txnRecord.walletId(),
                    details: txnRecord.confirmDetails(),
                    signedTransaction: signedTransaction
                )

            app.pushRoute(route)
        } catch {
            alertState = .init(.fileError(error.localizedDescription))
        }
    }

    func handleScanned(_: NfcMessage?, _ txn: NfcMessage?) {
        Log.debug("handleScanned")
        guard let txn else { return }

        do {
            let bitcoinTransaction = try BitcoinTransaction.tryFromNfcMessage(nfcMessage: txn)
            let db = Database().unsignedTransactions()
            let txnRecord = try db.getTxThrow(txId: bitcoinTransaction.txId())

            let route = RouteFactory()
                .sendConfirm(
                    id: txnRecord.walletId(),
                    details: txnRecord.confirmDetails(),
                    signedTransaction: bitcoinTransaction
                )

            app.pushRoute(route)
        } catch {
            Log.error("Failed to handle scanned transaction: \(error), txn: \(txn)")
            alertState = .init(.nfcError(error.localizedDescription))
        }
    }

    func txnRecordAndSignedTxn(_ hex: String) throws -> (
        UnsignedTransactionRecord, BitcoinTransaction
    ) {
        Log.info("txnRecordAndSignedTxn")
        let bitcoinTransaction = try BitcoinTransaction(txHex: hex)
        let db = Database().unsignedTransactions()
        let record = try db.getTxThrow(txId: bitcoinTransaction.txId())
        return (record, bitcoinTransaction)
    }

    @ViewBuilder
    var AccountSection: some View {
        VStack {
            HStack {
                BitcoinShieldIcon(width: 24, color: .orange)

                VStack(alignment: .leading, spacing: 6) {
                    Text(metadata.identOrFingerprint())
                        .font(.caption)
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
                        .background(.btnPrimary)
                        .cornerRadius(10)
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
                        .background(.btnPrimary)
                        .cornerRadius(10)
                        .font(.caption)
                        .fontWeight(.medium)
                }
            }
        }
    }

    @ViewBuilder
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
                Log.error("Failed to convert PSBT to BBQR: \(error)")
                alertState = .init(.bbqrError(error.localizedDescription))
            }
        }

        Button("NFC") {
            app.nfcWriter.writeToTag(data: details.psbtBytes())
        }

        ShareLink(
            item: PSBTFile(data: details.psbtBytes(), filename: "transaction.psbt"),
            preview: SharePreview(
                "transaction.psbt - A Partially Signed Bitcoin Transaction",
                image: Image(.bitcoinShield)
            )
        ) {
            Text("More...")
        }
    }

    @ViewBuilder
    var ImportTransactionDialog: some View {
        Button("QR") {
            app.sheetState = .init(.qr)
        }

        Button("File") {
            isPresentingFilePicker = true
        }

        Button("Paste") {
            let code = UIPasteboard.general.string ?? ""
            guard !code.isEmpty else {
                alertState = .init(.pasteError("No text found on the clipboard."))
                return
            }

            do {
                let (txnRecord, signedTransaction) = try txnRecordAndSignedTxn(code)
                let route = RouteFactory()
                    .sendConfirm(
                        id: txnRecord.walletId(), details: txnRecord.confirmDetails(),
                        signedTransaction: signedTransaction
                    )
                app.pushRoute(route)
            } catch {
                alertState = .init(.pasteError(error.localizedDescription))
            }
        }

        Button("NFC") {
            app.nfcReader.scan()
        }
    }

    // MARK: Sheet

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .details:
            SendFlowDetailsSheetView(manager: manager, details: details)
                .presentationDetents([.height(425), .height(600), .large])
                .padding()
        case .inputOutputDetails:
            SendFlowAdvancedDetailsView(manager: manager, details: details)
                .presentationDetents(
                    [.height(300), .height(400), .height(500), .large],
                    selection: $inputOutputDetailsPresentationSize
                )
        case let .exportQr(qrs):
            SendFlowBbqrExport(qrs: qrs.map { QrCodeView(text: $0) })
                .presentationDetents([.height(425), .height(600), .large])
                .padding()
                .padding(.top, 10)
        }
    }

    // MARK: Alerts

    var showingAlert: Binding<Bool> {
        Binding(
            get: { alertState != nil },
            set: { if !$0 { alertState = .none } }
        )
    }

    private var alertTitle: String {
        guard let alertState else { return "Error" }
        return MyAlert(alertState).title
    }

    private func MyAlert(_ alert: TaggedItem<AlertState>) -> some AlertBuilderProtocol {
        let singleOkCancel = {
            Button("Ok", role: .cancel) {
                alertState = .none
            }
        }

        switch alert.item {
        case let .bbqrError(message):
            return AlertBuilder(
                title: "QR Error",
                message: "Unable to create BBQr: \(message)",
                actions: singleOkCancel
            )
        case let .fileError(message):
            return AlertBuilder(
                title: "File Import Error",
                message: message,
                actions: singleOkCancel
            )
        case let .nfcError(error):
            return AlertBuilder(
                title: "NFC Error",
                message: error,
                actions: singleOkCancel
            )
        case let .pasteError(error):
            return AlertBuilder(
                title: "Paste Error",
                message: error,
                actions: singleOkCancel
            )
        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowHardwareScreen(
            id: WalletId(),
            manager: WalletManager(preview: "preview_only"),
            details: ConfirmDetails.previewNew()
        )
        .environment(AppManager.shared)
    }
}
