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
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

            // MARK: CONTENT

            ZStack {
                ScrollView {
                    VStack(spacing: 24) {
                        // Set amount, header and text
                        SendFlowAmountInfoSection()

                        // Amount input
                        SendFlowCoinControlAmountSection(
                            totalSending: totalSending,
                            sendAmountFiat: sendFlowManager.sendAmountFiat,
                            unit: manager.unit,
                            offset: offset,
                            canEditCustomAmount: sendFlowManager.feeSelection != nil,
                            showCustomAmount: showCustomAmount,
                            updateUnit: { manager.dispatch(action: .updateUnit($0)) }
                        )

                        // Address Section
                        VStack {
                            Divider()
                            EnterAddressView(address: sendFlowManager.enteringAddress)
                            Divider()
                        }

                        // Account Section
                        SendFlowAccountSection(manager: manager, showsTitle: true)

                        if sendFlowManager.feeSelection != nil,
                           sendFlowManager.address != nil
                        {
                            // Network Fee Section
                            SendFlowNetworkFeeSection(
                                selectedFeeRate: selectedFeeRate,
                                totalFeeString: totalFeeString,
                                showFeeSelection: showFeeSelection
                            )

                            // Total Spending Section
                            SendFlowCoinControlTotalSpendingSection(
                                utxoCount: utxos.count,
                                totalSpentBtc: totalSpentBtc,
                                totalSpentInFiat: totalSpentInFiat,
                                showCustomAmount: showCustomAmount
                            )

                            // Next Button
                            SendFlowNextButton(action: next)
                        }
                    }

                    .toolbar {
                        ToolbarItemGroup(placement: .keyboard) {
                            SendFlowCoinControlToolbar(
                                focusField: presenter.focusField,
                                addressIsEmpty: sendFlowManager.enteringAddress.wrappedValue.isEmpty,
                                addressIsValid: validateAddress(),
                                amountIsValid: validateAmount(),
                                pasteAddress: pasteAddress,
                                focusAmount: { presenter.focusField = .amount },
                                showQrScanner: { presenter.sheetState = TaggedItem(.qr) },
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
                    ZStack {
                        Rectangle()
                            .fill(.black)
                            .opacity(loadingOpacity)
                            .ignoresSafeArea()

                        ProgressView()
                            .tint(.primary)
                            .opacity(loadingOpacity)
                    }
                }
            }
        }
        .padding(.top, 0)
        .onChange(of: presenter.focusField, initial: true, focusFieldChanged)
        .onChange(of: scannedCode, initial: false, scannedCodeChanged)
        .onChange(of: metadata.selectedUnit, initial: false) { oldUnit, newUnit in
            sendFlowManager.dispatch(.notifySelectedUnitedChanged(old: oldUnit, new: newUnit))
        }
        .onChange(of: app.prices, initial: true) { _, newPrices in
            guard let prices = newPrices else { return }
            sendFlowManager.dispatch(.notifyPricesChanged(prices))
        }
        .task {
            let isAlreadyValid = validate()
            if !isAlreadyValid || utxos == sendFlowManager.rust.utxos() {
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
            if !isAlreadyValid || utxos == sendFlowManager.rust.utxos() {
                try? await Task.sleep(for: .milliseconds(700))
            }

            await MainActor.run {
                if !isAlreadyValid { presenter.focusField = .address }
                if validate() { presenter.focusField = .none }
                if sendFlowManager.address != nil {
                    let _ = self.validateAddress(displayAlert: true)
                }
            }
        }
        .onAppear {
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
                return
            }
        }
        .sheet(item: presenter.sheetStateBinding, content: SheetContent)
        .sheet(isPresented: $customAmountSheetIsPresented) {
            SendFlowUtxoCustomAmountSheetView(utxos: utxos)
        }
        .alert(
            presenter.alertTitle,
            isPresented: presenter.showingAlert,
            presenting: presenter.alertState,
            actions: presenter.alertButtons,
            message: presenter.alertMessage
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

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .qr:
            QrCodeAddressView(app: _app, scannedCode: $scannedCode)
                .presentationDetents([.large])
        case .fee:
            if let localFeeSelection = sendFlowManager.feeSelection {
                SendFlowSelectFeeRateView(
                    manager: manager,
                    feeOptions: Binding(
                        get: { sendFlowManager.feeSelection?.options ?? localFeeSelection.options },
                        set: { sendFlowManager.dispatch(action: .changeFeeRateOptions($0)) }
                    ),
                    selectedOption: Binding(
                        get: {
                            sendFlowManager.feeSelection?.selected ?? localFeeSelection.selected
                        },
                        set: { newValue in
                            sendFlowManager.dispatch(action: .selectFeeRate(newValue))
                        }
                    ),
                    selectedPresentationDetent: $selectedPresentationDetent
                )
                .presentationDetents(
                    [.height(440), .height(550), .large],
                    selection: $selectedPresentationDetent
                )
            }
        }
    }
}

#Preview {
    AsyncPreview {
        NavigationStack {
            let manager = WalletManager(preview: "preview_only")
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
