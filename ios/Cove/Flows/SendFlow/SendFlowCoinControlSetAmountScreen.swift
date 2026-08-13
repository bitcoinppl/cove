//
//  SendFlowCoinControlSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import CoveCore
import Foundation
import SwiftUI

// MARK: SendFlowCoinControlSetAmountScreen

private typealias FocusField = SendFlowPresenter.FocusField
private typealias SheetState = SendFlowPresenter.SheetState
private typealias AlertState = SendFlowAlertState

struct SendFlowCoinControlSetAmountScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(SendFlowManager.self) private var sendFlowManager
    @Environment(WalletManager.self) private var manager

    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId
    let utxos: [Utxo]

    @State private var scannedCode: TaggedString? = .none

    /// fees
    @State private var selectedPresentationDetent: PresentationDetent = .height(440)

    // loading
    @State private var isLoading: Bool = true
    @State private var loadingOpacity: CGFloat = 1

    /// custom utxo amount
    @State private var customAmountSheetIsPresented: Bool = false

    private var presenter: SendFlowPresenter {
        sendFlowManager.presenter
    }

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    private var network: Network {
        metadata.network
    }

    private var totalSpentInFiat: String {
        sendFlowManager.totalSpentInFiat
    }

    private var totalFeeString: String? {
        sendFlowManager.totalFeeString
    }

    private var totalSpentBtc: String {
        sendFlowManager.totalSpentInBtc
    }

    private var totalSending: String {
        sendFlowManager.sendAmountBtc
    }

    // MARK: Actions

    /// validate, create final psbt and send to next screen
    private func next() {
        if validate(true) { sendFlowManager.dispatch(action: .finalizeAndGoToNextScreen) }
    }

    private func dismissIfValid() {
        if validate(true) { presenter.focusField = .none }
    }

    /// doing it this way prevents an alert popping up when the user just goes back
    private func setAlertState(_ error: SendFlowError) {
        sendFlowManager.presenter.alertState = .init(.error(error))
    }

    var selectedFeeRate: FeeRateOptionWithTotalFee? {
        sendFlowManager.selectedFeeRate
    }

    var feeRateOptions: FeeRateOptionsWithTotalFee? {
        sendFlowManager.feeRateOptions
    }

    var offset: CGFloat {
        if metadata.fiatOrBtc == .fiat { return 0 }
        return metadata.selectedUnit == .btc ? screenWidth * 0.09 : screenWidth * 0.10
    }

    var body: some View {
        SendFlowCoinControlContent(
            utxos: utxos,
            isLoading: isLoading,
            loadingOpacity: loadingOpacity,
            next: next,
            dismissIfValid: dismissIfValid,
            showCustomAmount: showCustomAmount,
            showFeeSelection: showFeeSelection
        )
        .padding(.top, 0)
        .onChange(of: presenter.focusField, initial: true, focusFieldChanged)
        .onChange(of: scannedCode, initial: false, scannedCodeChanged)
        .onChange(of: metadata.selectedUnit, initial: false, selectedUnitChanged)
        .onChange(of: app.prices, initial: true, pricesChanged)
        .task { await prepareScreen() }
        .onAppear(perform: screenAppeared)
        .sheet(item: presenter.sheetStateBinding) { state in
            SendFlowCoinControlSheetContent(
                state: state.item,
                scannedCode: $scannedCode,
                selectedPresentationDetent: $selectedPresentationDetent
            )
        }
        .sheet(isPresented: $customAmountSheetIsPresented) {
            SendFlowUtxoCustomAmountSheetView(utxos: utxos)
        }
        .presentingAlert(
            presenter.alertStateBinding,
            context: SendFlowAlertContext(
                presenter: presenter,
                sendFlowManager: sendFlowManager
            )
        )
    }

    // MARK: Validation Functions

    private func validate(_ displayAlert: Bool = false) -> Bool {
        sendFlowManager.validate(displayAlert: displayAlert)
    }

    private func validateAmount(displayAlert: Bool = false) -> Bool {
        sendFlowManager.validateAmount(displayAlert: displayAlert)
    }

    private func validateAddress(displayAlert: Bool = false) -> Bool {
        sendFlowManager.validateAddress(displayAlert: displayAlert)
    }

    // MARK: OnChange Functions

    private func selectedUnitChanged(oldUnit: Unit, newUnit: Unit) {
        Log.debug("selectedUnitChanged \(oldUnit) -> \(newUnit)")
        sendFlowManager.dispatch(action: .notifySelectedUnitedChanged(old: oldUnit, new: newUnit))
    }

    private func pricesChanged(_: PriceResponse?, _ newPrices: PriceResponse?) {
        guard let newPrices else { return }

        sendFlowManager.dispatch(.notifyPricesChanged(newPrices))
    }

    private func prepareScreen() async {
        let isAlreadyValid = validate()
        let shouldShowLoading = !isAlreadyValid || utxos == sendFlowManager.rust.utxos()

        if shouldShowLoading {
            Task {
                await MainActor.run {
                    withAnimation(
                        .easeInOut(duration: 1.5).delay(0.4),
                        completionCriteria: .removed
                    ) {
                        loadingOpacity = 0
                    } completion: {
                        isLoading = false
                        if validate() { presenter.focusField = .none }
                    }
                }
            }
        } else {
            presenter.focusField = .none
        }

        // HACK: Bug in SwiftUI where keyboard toolbar is broken
        if shouldShowLoading {
            try? await Task.sleep(for: .milliseconds(700))
        }

        await MainActor.run {
            if !isAlreadyValid { presenter.focusField = .address }
            if validate() { presenter.focusField = .none }
            if sendFlowManager.address != nil {
                _ = validateAddress(displayAlert: true)
            }
        }
    }

    private func screenAppeared() {
        sendFlowManager.dispatch(.setCoinControlMode(utxos))
        if validate(), utxos == sendFlowManager.rust.utxos() {
            isLoading = false
            loadingOpacity = 0
            presenter.focusField = .none
        } else {
            presenter.focusField = .address
        }

        if metadata.walletType == .watchOnly {
            app.alertState = .init(.cantSendOnWatchOnlyWallet)
            app.popRoute()
        }
    }

    /// presenter focus field changed
    private func focusFieldChanged(_ oldField: FocusField?, _ newField: FocusField?) {
        Log.debug(
            "focusFieldChanged \(String(describing: oldField)) -> \(String(describing: newField))"
        )

        sendFlowManager.dispatch(action: .notifyFocusFieldChanged(old: oldField, new: newField))
    }

    private func clearAddress() {
        Log.debug("clearAddress")
        sendFlowManager.dispatch(action: .clearAddress)
    }

    private func pasteAddress() {
        let address = UIPasteboard.general.string ?? ""
        sendFlowManager.dispatch(action: .changeEnteringAddress(address))
    }

    private func showCustomAmount() {
        customAmountSheetIsPresented = true
    }

    private func showFeeSelection() {
        selectedPresentationDetent =
            if feeRateOptions?.custom() == nil { .height(440) } else { .height(550) }
        presenter.sheetState = TaggedItem(.fee)
    }

    private func scannedCodeChanged(old: TaggedString?, newValue: TaggedString?) {
        Log.debug(
            "scannedCodeChanged \(String(describing: old)) -> \(String(describing: newValue))"
        )
        guard let newValue else { return }
        presenter.sheetState = nil
        sendFlowManager.dispatch(
            action: .notifyScanCodeChanged(old: old?.item ?? "", new: newValue.item)
        )
    }
}

private struct SendFlowCoinControlContent: View {
    @Environment(SendFlowManager.self) private var sendFlowManager
    @Environment(WalletManager.self) private var manager
    @Environment(\.colorScheme) private var colorScheme

    let utxos: [Utxo]
    let isLoading: Bool
    let loadingOpacity: CGFloat
    let next: () -> Void
    let dismissIfValid: () -> Void
    let showCustomAmount: () -> Void
    let showFeeSelection: () -> Void

    private var presenter: SendFlowPresenter {
        sendFlowManager.presenter
    }

    private var offset: CGFloat {
        let metadata = manager.walletMetadata
        if metadata.fiatOrBtc == .fiat { return 0 }

        return metadata.selectedUnit == .btc ? screenWidth * 0.09 : screenWidth * 0.10
    }

    var body: some View {
        VStack(spacing: 0) {
            SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

            ZStack {
                ScrollView {
                    VStack(spacing: 24) {
                        SendFlowAmountInfoSection()

                        SendFlowCoinControlAmountSection(
                            totalSending: sendFlowManager.sendAmountBtc,
                            sendAmountFiat: sendFlowManager.sendAmountFiat,
                            unit: manager.unit,
                            offset: offset,
                            canEditCustomAmount: sendFlowManager.feeSelection != nil,
                            showCustomAmount: showCustomAmount,
                            updateUnit: updateUnit
                        )

                        SendFlowCoinControlAddressSection(
                            address: sendFlowManager.enteringAddress
                        )
                        SendFlowAccountSection(manager: manager, showsTitle: true)

                        if sendFlowManager.feeSelection != nil,
                           sendFlowManager.address != nil
                        {
                            SendFlowNetworkFeeSection(
                                selectedFeeRate: sendFlowManager.selectedFeeRate,
                                totalFeeString: sendFlowManager.totalFeeString,
                                showFeeSelection: showFeeSelection
                            )

                            SendFlowCoinControlTotalSpendingSection(
                                utxoCount: utxos.count,
                                totalSpentBtc: sendFlowManager.totalSpentInBtc,
                                totalSpentInFiat: sendFlowManager.totalSpentInFiat,
                                showCustomAmount: showCustomAmount
                            )

                            SendFlowNextButton(action: next)
                        }
                    }
                    .toolbar {
                        ToolbarItemGroup(placement: .keyboard) {
                            SendFlowCoinControlToolbar(
                                focusField: presenter.focusField,
                                addressIsEmpty: sendFlowManager.enteringAddress.wrappedValue.isEmpty,
                                addressIsValid: sendFlowManager.validateAddress(),
                                amountIsValid: sendFlowManager.validateAmount(),
                                pasteAddress: pasteAddress,
                                focusAmount: focusAmount,
                                showQrScanner: showQrScanner,
                                clearAddress: clearAddress,
                                dismissIfValid: dismissIfValid
                            )
                        }
                    }
                }
                .padding(.horizontal)
                .frame(maxWidth: .infinity)
                .background(colorScheme == .light ? .white : .black)
                .scrollIndicators(.hidden)

                if isLoading {
                    SendFlowCoinControlLoadingOverlay(opacity: loadingOpacity)
                }
            }
        }
    }

    private func updateUnit(_ unit: Unit) {
        manager.dispatch(action: .updateUnit(unit))
    }

    private func pasteAddress() {
        let address = UIPasteboard.general.string ?? ""
        sendFlowManager.dispatch(action: .changeEnteringAddress(address))
    }

    private func focusAmount() {
        presenter.focusField = .amount
    }

    private func showQrScanner() {
        presenter.sheetState = TaggedItem(.qr)
    }

    private func clearAddress() {
        sendFlowManager.dispatch(action: .clearAddress)
    }
}

private struct SendFlowCoinControlAddressSection: View {
    let address: Binding<String>

    var body: some View {
        VStack {
            Divider()
            EnterAddressView(address: address)
            Divider()
        }
    }
}

private struct SendFlowCoinControlLoadingOverlay: View {
    let opacity: CGFloat

    var body: some View {
        ZStack {
            Rectangle()
                .fill(.black)
                .opacity(opacity)
                .ignoresSafeArea()

            ProgressView()
                .tint(.primary)
                .opacity(opacity)
        }
    }
}

private struct SendFlowCoinControlSheetContent: View {
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var manager
    @Environment(SendFlowManager.self) private var sendFlowManager

    let state: SheetState
    @Binding var scannedCode: TaggedString?
    @Binding var selectedPresentationDetent: PresentationDetent

    var body: some View {
        switch state {
        case .qr:
            QrCodeAddressView(app: _app, scannedCode: $scannedCode)
                .presentationDetents([.large])
        case .fee:
            if let feeSelection = sendFlowManager.feeSelection {
                SendFlowCoinControlFeeSelectionSheet(
                    manager: manager,
                    sendFlowManager: sendFlowManager,
                    feeSelection: feeSelection,
                    selectedPresentationDetent: $selectedPresentationDetent
                )
            }
        }
    }
}

private struct SendFlowCoinControlFeeSelectionSheet: View {
    let manager: WalletManager
    let sendFlowManager: SendFlowManager
    let feeSelection: FeeSelection
    @Binding var selectedPresentationDetent: PresentationDetent

    var body: some View {
        SendFlowSelectFeeRateView(
            manager: manager,
            feeOptions: Binding(
                get: { sendFlowManager.feeSelection?.options ?? feeSelection.options },
                set: { sendFlowManager.dispatch(action: .changeFeeRateOptions($0)) }
            ),
            selectedOption: Binding(
                get: { sendFlowManager.feeSelection?.selected ?? feeSelection.selected },
                set: { sendFlowManager.dispatch(action: .selectFeeRate($0)) }
            ),
            selectedPresentationDetent: $selectedPresentationDetent
        )
        .presentationDetents(
            [.height(440), .height(550), .large],
            selection: $selectedPresentationDetent
        )
    }
}

#Preview {
    AsyncPreview {
        NavigationStack {
            let manager = WalletManager(preview: .only)
            let presenter = SendFlowPresenter(app: AppManager.shared, manager: manager)

            if let rustSendFlowManager = try? manager.rust.newSendFlowManager(balance: manager.balance) {
                let sendFlowManager = SendFlowManager(rustSendFlowManager, presenter: presenter)

                SendFlowCoinControlSetAmountScreen(
                    id: WalletId(), utxos: previewNewUtxoList(outputCount: 15, changeCount: 3)
                )
                .environment(manager)
                .environment(AppManager.shared)
                .environment(presenter)
                .environment(sendFlowManager)
            }
        }
    }
}
