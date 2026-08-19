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

private enum SendFlowHardwarePresentation: TaggedAlertPresentable, TaggedSheetPresentable {
    case confirmation(ConfirmationState)
    case sheet(SendFlowHardwareSheetState)
    case alert(SendFlowHardwareAlertState)
    case fileImporter
    case qrScanner
    case shareTransaction
    case exportNfc
    case pasteSignature
    case scanNfc

    func sheet(context: SendFlowHardwarePresentationContext) -> AnyView {
        guard case let .sheet(sheet) = self else {
            preconditionFailure("A non-sheet hardware presentation reached the sheet host")
        }

        return sheet.sheet(context: context)
    }

    func alert(context: SendFlowHardwarePresentationContext) -> AnyAlertBuilder {
        guard case let .alert(alert) = self else {
            preconditionFailure("A non-alert hardware presentation reached the alert host")
        }

        return alert.alert(context: context)
    }
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

    /// sheets, alerts, confirmations
    @State private var presentationCoordinator =
        PresentationTransitionCoordinator<SendFlowHardwarePresentation>()
    @State private var inputOutputDetailsPresentationSize: PresentationDetent = .height(300)

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
            dismissAlert: presentationCoordinator.dismissCurrentPresentation,
            inputOutputDetailsPresentationSize: $inputOutputDetailsPresentationSize
        )
    }

    private var alertPresentation: Binding<TaggedItem<SendFlowHardwarePresentation>?> {
        presentationCoordinator.presentedItem { presentation in
            if case .alert = presentation { true } else { false }
        }
    }

    private var sheetPresentation: Binding<TaggedItem<SendFlowHardwarePresentation>?> {
        presentationCoordinator.presentedItem { presentation in
            if case .sheet = presentation { true } else { false }
        }
    }

    private var fileImporterIsPresented: Binding<Bool> {
        presentationCoordinator.isPresented { presentation in
            if case .fileImporter = presentation { true } else { false }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

            ScrollView {
                SendFlowHardwareTransactionContent(
                    manager: manager,
                    details: details,
                    fiatAmount: fiatAmount,
                    showInputOutputDetails: showInputOutputDetails,
                    exportTransaction: exportTransaction,
                    importSignature: importSignature,
                    exportDialogIsPresented: confirmationBinding(for: .exportTxn),
                    importDialogIsPresented: confirmationBinding(for: .importSignature),
                    dialogActions: SendFlowHardwareDialogActions(
                        exportQr: exportQr,
                        exportNfc: exportNfc,
                        shareTransaction: shareTransaction,
                        scanQr: scanQr,
                        importFile: importFile,
                        pasteSignature: queuePastedSignatureImport,
                        scanNfc: scanNfc
                    ),
                    showDetails: showDetails
                )
            }
            .scrollIndicators(.hidden)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal)
            .background(Color.background)
            .presentingSheet(sheetPresentation, context: presentationContext)
            .presentingAlert(alertPresentation, context: presentationContext, defaultTitle: "Error")
            .fileImporter(
                isPresented: fileImporterIsPresented,
                allowedContentTypes: [.plainText, .psbt, .txn],
                onCompletion: handleFileImport
            )
            .onAppear(perform: updateInputOutputDetailsPresentationSize)
            .toolbar { Toolbar }
        }
        .presentationTransitionHost(presentationCoordinator)
        .onChange(of: presentationCoordinator.currentPresentation?.id, initial: true) {
            _, _ in
            performCurrentPresentationAction()
        }
        .onChange(of: app.sheetState?.id) { _, _ in
            completeQrScannerIfNeeded()
        }
    }

    private func showInputOutputDetails() {
        presentationCoordinator.present(.sheet(.inputOutputDetails))
    }

    private func exportTransaction() {
        presentationCoordinator.present(.confirmation(.exportTxn))
    }

    private func importSignature() {
        presentationCoordinator.present(.confirmation(.importSignature))
    }

    private func showDetails() {
        presentationCoordinator.present(.sheet(.details))
    }

    private func updateInputOutputDetailsPresentationSize() {
        let total = details.outputs().count + details.inputs().count
        if total == 3 { inputOutputDetailsPresentationSize = .height(300) }
        if total > 3 { inputOutputDetailsPresentationSize = .height(400) }
        if total > 5 { inputOutputDetailsPresentationSize = .height(500) }
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
            presentationCoordinator.present(.alert(.fileError(error.localizedDescription)))
        }
    }

    func importPastedSignature() {
        let code = UIPasteboard.general.string ?? ""
        guard !code.isEmpty else {
            presentationCoordinator.present(
                .alert(.pasteError("No text found on the clipboard."))
            )
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
            presentationCoordinator.present(.alert(.pasteError(error.localizedDescription)))
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
            presentationCoordinator.present(.alert(.nfcError(error.localizedDescription)))
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

    private func confirmationBinding(for state: ConfirmationState) -> Binding<Bool> {
        presentationCoordinator.isPresented { presentation in
            guard case let .confirmation(currentState) = presentation else {
                return false
            }

            return currentState == state
        }
    }

    private func exportQr() {
        presentationCoordinator.transition(to: .sheet(.exportQr))
    }

    private func exportNfc() {
        presentationCoordinator.transition(to: .exportNfc)
    }

    private func shareTransaction() {
        presentationCoordinator.transition(to: .shareTransaction)
    }

    private func scanQr() {
        presentationCoordinator.transition(to: .qrScanner)
    }

    private func importFile() {
        presentationCoordinator.transition(to: .fileImporter)
    }

    private func queuePastedSignatureImport() {
        presentationCoordinator.transition(to: .pasteSignature)
    }

    private func scanNfc() {
        presentationCoordinator.transition(to: .scanNfc)
    }

    private func performCurrentPresentationAction() {
        guard let presentation = presentationCoordinator.currentPresentation?.item else { return }

        switch presentation {
        case .confirmation, .sheet, .alert, .fileImporter:
            return
        case .qrScanner:
            app.sheetState = .init(.qr)
        case .shareTransaction:
            ShareSheet.present(
                data: details.psbtBytes(),
                filename: "transaction.psbt"
            ) { _ in
                presentationCoordinator.dismissCurrentPresentation()
            }
        case .exportNfc:
            app.nfcWriter.writeToTag(data: details.psbtBytes())
            presentationCoordinator.dismissCurrentPresentation()
        case .pasteSignature:
            presentationCoordinator.dismissCurrentPresentation()
            importPastedSignature()
        case .scanNfc:
            app.nfcReader.scan()
            presentationCoordinator.dismissCurrentPresentation()
        }
    }

    private func completeQrScannerIfNeeded() {
        guard case .qrScanner = presentationCoordinator.currentPresentation?.item else { return }
        guard app.sheetState == nil else { return }

        presentationCoordinator.dismissCurrentPresentation()
    }
}

private struct SendFlowHardwareTransactionContent: View {
    @Bindable var manager: WalletManager

    let details: ConfirmDetails
    let fiatAmount: String
    let showInputOutputDetails: () -> Void
    let exportTransaction: () -> Void
    let importSignature: () -> Void
    let exportDialogIsPresented: Binding<Bool>
    let importDialogIsPresented: Binding<Bool>
    let dialogActions: SendFlowHardwareDialogActions
    let showDetails: () -> Void

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var body: some View {
        VStack(spacing: 24) {
            SendFlowHardwareAmountSummary(
                manager: manager,
                details: details,
                fiatAmount: fiatAmount
            )

            SendFlowHardwareAccountSection(metadata: metadata)
                .padding(.vertical)

            Divider()

            SendFlowHardwareAddressRow(
                address: details.sendingTo(),
                showInputOutputDetails: showInputOutputDetails
            )

            Divider()

            if case let .tapSigner(tapSigner) = metadata.hardwareMetadata {
                SendFlowTapSignerTransactionSection(
                    tapSigner: tapSigner,
                    psbt: details.psbt()
                )
            } else {
                SendFlowHardwareSignTransactionSection(
                    exportTransaction: exportTransaction,
                    importSignature: importSignature,
                    exportDialogIsPresented: exportDialogIsPresented,
                    importDialogIsPresented: importDialogIsPresented,
                    dialogActions: dialogActions
                )
            }

            Spacer()

            Button("More details", action: showDetails)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fontWeight(.medium)
        }
    }
}

private struct SendFlowHardwareAmountSummary: View {
    @Bindable var manager: WalletManager

    let details: ConfirmDetails
    let fiatAmount: String

    var body: some View {
        VStack(spacing: 24) {
            SendFlowHardwareAmountTitle()
                .padding(.top)

            SendFlowHardwareAmount(
                amount: manager.amountFmt(details.sendingAmount()),
                selectedUnit: manager.walletMetadata.selectedUnit,
                fiatAmount: fiatAmount,
                selectSats: selectSats,
                selectBtc: selectBtc
            )
        }
    }

    private func selectSats() {
        manager.dispatch(action: .updateUnit(.sat))
    }

    private func selectBtc() {
        manager.dispatch(action: .updateUnit(.btc))
    }
}

private struct SendFlowHardwareAmountTitle: View {
    var body: some View {
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
    }
}

private struct SendFlowHardwareAmount: View {
    let amount: String
    let selectedUnit: Unit
    let fiatAmount: String
    let selectSats: () -> Void
    let selectBtc: () -> Void

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                Text(amount)
                    .font(.system(size: 48, weight: .bold))
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)

                SendFlowHardwareUnitMenu(
                    selectedUnit: selectedUnit,
                    selectSats: selectSats,
                    selectBtc: selectBtc
                )
                .offset(y: -5)
                .offset(x: -16)
            }
            .offset(x: 32)

            Text(fiatAmount)
                .font(.title3)
                .foregroundColor(.secondary)
        }
        .padding(.top, 8)
    }
}

private struct SendFlowHardwareUnitMenu: View {
    let selectedUnit: Unit
    let selectSats: () -> Void
    let selectBtc: () -> Void

    var body: some View {
        Text(selectedUnit == .sat ? "sats" : "btc")
            .padding(.vertical, 10)
            .padding(.horizontal, 16)
            .contentShape(
                .contextMenuPreview,
                RoundedRectangle(cornerRadius: 8)
            )
            .contextMenu {
                Button("sats", action: selectSats)
                Button("btc", action: selectBtc)
            } preview: {
                Text(selectedUnit == .sat ? "sats" : "btc")
                    .padding(.vertical, 10)
                    .padding(.horizontal)
            }
    }
}

private struct SendFlowHardwareAddressRow: View {
    let address: Address
    let showInputOutputDetails: () -> Void

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

            Text(address.spacedOut())
                .lineLimit(4, reservesSpace: false)
                .font(.system(.footnote, design: .none))
                .fontWeight(.semibold)
                .padding(16)
                .contentShape(
                    .contextMenuPreview,
                    RoundedRectangle(cornerRadius: 8)
                )
                .contextMenu {
                    Button("Copy", systemImage: "doc.on.doc", action: copyAddress)
                }
                .padding(-16)
                .padding(.leading, 50)
        }
        .padding(.vertical, 8)
        .onTapGesture(perform: showInputOutputDetails)
    }

    private func copyAddress() {
        UIPasteboard.general.string = address.unformatted()
    }
}

private struct SendFlowTapSignerTransactionSection: View {
    @Environment(AppManager.self) private var app

    let tapSigner: TapSigner
    let psbt: Psbt

    var body: some View {
        VStack(spacing: 17) {
            HStack {
                Text("Sign Transaction")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundColor(.secondary)

                Spacer()
            }

            Button(action: signTransaction) {
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

    private func signTransaction() {
        let route = TapSignerRoute.enterPin(tapSigner: tapSigner, action: .sign(psbt))
        app.sheetState = .init(.tapSigner(route))
    }
}

#Preview {
    AsyncPreview {
        SendFlowHardwareScreen(
            id: WalletId(),
            manager: WalletManager(preview: .only),
            details: confirmDetailsPreviewNew()
        )
        .environment(AppManager.shared)
    }
}
