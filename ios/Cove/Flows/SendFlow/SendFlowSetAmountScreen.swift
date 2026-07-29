//
//  SendFlowSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import CoveCore
import Foundation
import SwiftUI

// MARK: SendFlowSetAmountScreen

private typealias FocusField = SendFlowPresenter.FocusField
private typealias SheetState = SendFlowPresenter.SheetState
private typealias AlertState = SendFlowAlertState

struct SendFlowSetAmountScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(SendFlowManager.self) private var sendFlowManager
    @Environment(WalletManager.self) private var manager

    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId

    @State var amount: Amount? = nil

    // private
    @State private var isLoading: Bool = true
    @State private var loadingOpacity: CGFloat = 1

    @State private var scrollPosition: ScrollPosition = .init(
        idType: SendFlowPresenter.FocusField.self
    )

    @State private var scannedCode: TaggedString? = .none

    /// fees
    @State private var selectedPresentationDetent: PresentationDetent = .height(440)

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

    private var address: Binding<String> {
        sendFlowManager.enteringAddress
    }

    private var totalSending: String {
        sendFlowManager.sendAmountBtc
    }

    // MARK: Actions

    private func next() {
        Task {
            guard await performValidation() else { return }
            sendFlowManager.dispatch(action: .finalizeAndGoToNextScreen)
        }
    }

    private func dismissIfValid() {
        Task {
            guard await performValidation() else { return }
            presenter.focusField = .none
        }
    }

    private func performValidation() async -> Bool {
        if !validateAddress() {
            if !address.wrappedValue.isEmpty {
                await FloaterPopup(
                    text: "Address not valid. Please try again.",
                    backgroundColor: .orange,
                    textColor: .white,
                    iconColor: .white,
                    icon: "exclamationmark.triangle"
                ).dismissAfter(1).present()
            }
            presenter.focusField = .address
            return false
        }
        if !validateAmount() {
            let hasAmount = sendFlowManager.amount != nil && sendFlowManager.amount?.asSats() != 0
            if hasAmount {
                await FloaterPopup(
                    text: "Amount not valid. Please try again.",
                    backgroundColor: .orange,
                    textColor: .white,
                    iconColor: .white,
                    icon: "exclamationmark.triangle"
                ).dismissAfter(1).present()
            }
            presenter.focusField = .amount
            return false
        }
        return true
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

    var body: some View {
        SendFlowSetAmountContent(
            isLoading: isLoading,
            loadingOpacity: loadingOpacity,
            scrollPosition: $scrollPosition,
            next: next,
            dismissIfValid: dismissIfValid,
            showFeeSelection: showFeeSelection
        )
        .padding(.top, 0)
        .onChange(of: presenter.focusField, initial: true, focusFieldChanged)
        .onChange(of: scannedCode, initial: false, scannedCodeChanged)
        .onChange(of: metadata.selectedUnit, initial: false, selectedUnitChanged)
        .onChange(of: metadata.fiatOrBtc, initial: false, fiatOrBtcChanged)
        .onChange(of: app.prices, initial: true, pricesChanged)
        .task { await prepareScreen() }
        .onAppear(perform: screenAppeared)
        .sheet(item: presenter.sheetStateBinding) { state in
            SendFlowSetAmountSheetContent(
                state: state.item,
                scannedCode: $scannedCode,
                selectedPresentationDetent: $selectedPresentationDetent
            )
        }
        .presentingAlert(
            presenter.alertStateBinding,
            context: SendFlowAlertContext(
                presenter: presenter,
                sendFlowManager: sendFlowManager
            )
        )
    }

    private var totalFeeString: String? {
        sendFlowManager.totalFeeString
    }

    private var totalSpentBtc: String {
        sendFlowManager.totalSpentInBtc
    }

    private func clearSendAmount() {
        sendFlowManager.dispatch(action: .clearSendAmount)
    }

    private func fiatOrBtcChanged(_ oldValue: FiatOrBtc, _ newValue: FiatOrBtc) {
        sendFlowManager.dispatch(.notifyBtcOrFiatChanged(old: oldValue, new: newValue))
    }

    private func pricesChanged(_: PriceResponse?, _ newPrices: PriceResponse?) {
        guard let newPrices else { return }

        sendFlowManager.dispatch(.notifyPricesChanged(newPrices))
    }

    private func prepareScreen() async {
        let isAlreadyValid = validate()

        if !isAlreadyValid {
            Task {
                await MainActor.run {
                    withAnimation(
                        .easeInOut(duration: 1.5).delay(0.4),
                        completionCriteria: .removed
                    ) {
                        loadingOpacity = 0
                    } completion: {
                        isLoading = false
                    }
                }
            }
        }

        // HACK: Bug in SwiftUI where keyboard toolbar is broken
        if !isAlreadyValid { try? await Task.sleep(for: .milliseconds(700)) }

        await MainActor.run {
            Log.debug(
                "SendFlowSetAmountScreen: onAppear \(String(describing: sendFlowManager.amount))"
            )

            if sendFlowManager.amount == nil || sendFlowManager.amount?.asSats() == 0 {
                presenter.focusField = .amount
            } else if address.wrappedValue.isEmpty {
                presenter.focusField = .address
            }

            if let amount = sendFlowManager.amount, amount.asSats() != 0 {
                _ = validateAmount(displayAlert: true)
            }

            if sendFlowManager.address != nil {
                _ = validateAddress(displayAlert: true)
            }
        }
    }

    private func screenAppeared() {
        if validate() {
            isLoading = false
            loadingOpacity = 0
            presenter.focusField = .none
        }

        sendFlowManager.dispatch(action: .disableCoinControlMode)
        if metadata.walletType == .watchOnly {
            app.alertState = .init(.cantSendOnWatchOnlyWallet)
            app.popRoute()
        }
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

        guard let newField else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            withAnimation(.easeInOut(duration: 0.4)) {
                // if keyboard opening directly to amount, dont update scroll position
                if newField == .amount, oldField == .none { return }
                Log.debug("scrolling to \(String(describing: newField))")
                scrollPosition.scrollTo(id: newField)
            }
        }
    }

    private func setMaxSelected() {
        Log.debug("setMaxSelected")
        sendFlowManager.dispatch(action: .selectMaxSend)
    }

    private func clearAddress() {
        Log.debug("clearAddress")
        sendFlowManager.dispatch(action: .clearAddress)
    }

    private func pasteAddress() {
        let address = UIPasteboard.general.string ?? ""
        sendFlowManager.dispatch(action: .changeEnteringAddress(address))
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

private struct SendFlowSetAmountContent: View {
    @Environment(SendFlowManager.self) private var sendFlowManager
    @Environment(WalletManager.self) private var manager
    @Environment(\.colorScheme) private var colorScheme

    let isLoading: Bool
    let loadingOpacity: CGFloat
    let scrollPosition: Binding<ScrollPosition>
    let next: () -> Void
    let dismissIfValid: () -> Void
    let showFeeSelection: () -> Void

    private var presenter: SendFlowPresenter {
        sendFlowManager.presenter
    }

    var body: some View {
        VStack(spacing: 0) {
            SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

            ZStack {
                ScrollView {
                    VStack(spacing: 24) {
                        SendFlowAmountInfoSection()
                        EnterAmountView(sendFlowManager: sendFlowManager)
                        SendFlowAddressSection(address: sendFlowManager.enteringAddress)
                        SendFlowAccountSection(manager: manager, showsTitle: true)

                        if sendFlowManager.feeSelection != nil,
                           sendFlowManager.address != nil
                        {
                            SendFlowNetworkFeeSection(
                                selectedFeeRate: sendFlowManager.selectedFeeRate,
                                totalFeeString: sendFlowManager.totalFeeString,
                                showFeeSelection: showFeeSelection
                            )

                            SendFlowTotalSpendingSection(
                                totalSpentBtc: sendFlowManager.totalSpentInBtc,
                                totalSpentInFiat: sendFlowManager.totalSpentInFiat
                            )

                            SendFlowNextButton(action: next)
                        }
                    }
                    .toolbar {
                        ToolbarItemGroup(placement: .keyboard) {
                            SendFlowSetAmountToolbar(
                                focusField: presenter.focusField,
                                addressIsEmpty: sendFlowManager.enteringAddress.wrappedValue.isEmpty,
                                addressIsValid: sendFlowManager.validateAddress(),
                                amountIsValid: sendFlowManager.validateAmount(),
                                focusAddress: focusAddress,
                                focusAmount: focusAmount,
                                selectMax: selectMax,
                                clearAmount: clearAmount,
                                pasteAddress: pasteAddress,
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
                .scrollPosition(scrollPosition, anchor: .top)

                if isLoading {
                    SendFlowLoadingOverlay(opacity: loadingOpacity)
                }
            }
        }
    }

    private func focusAddress() {
        presenter.focusField = .address
    }

    private func focusAmount() {
        presenter.focusField = .amount
    }

    private func selectMax() {
        sendFlowManager.dispatch(action: .selectMaxSend)
    }

    private func clearAmount() {
        sendFlowManager.dispatch(action: .clearSendAmount)
    }

    private func pasteAddress() {
        let address = UIPasteboard.general.string ?? ""
        sendFlowManager.dispatch(action: .changeEnteringAddress(address))
    }

    private func showQrScanner() {
        presenter.sheetState = TaggedItem(.qr)
    }

    private func clearAddress() {
        sendFlowManager.dispatch(action: .clearAddress)
    }
}

private struct SendFlowAddressSection: View {
    let address: Binding<String>

    var body: some View {
        VStack {
            Divider()
            EnterAddressView(address: address)
            Divider()
        }
    }
}

private struct SendFlowLoadingOverlay: View {
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

private struct SendFlowSetAmountSheetContent: View {
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
            SendFlowFeeSelectionSheet(
                manager: manager,
                sendFlowManager: sendFlowManager,
                selectedPresentationDetent: $selectedPresentationDetent
            )
        }
    }
}

private struct SendFlowFeeSelectionSheet: View {
    let manager: WalletManager
    let sendFlowManager: SendFlowManager
    @Binding var selectedPresentationDetent: PresentationDetent

    var body: some View {
        SendFlowSelectFeeRateView(
            manager: manager,
            feeOptions: Binding(
                get: { sendFlowManager.feeSelection!.options },
                set: { sendFlowManager.dispatch(action: .changeFeeRateOptions($0)) }
            ),
            selectedOption: Binding(
                get: { sendFlowManager.feeSelection!.selected },
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

#Preview("with address") {
    AsyncPreview {
        NavigationStack {
            let manager = WalletManager(preview: "preview_only")

            SendFlowSetAmountScreen(
                id: WalletId()
            )
            .environment(manager)
            .environment(AppManager.shared)
            .environment(SendFlowPresenter(app: AppManager.shared, manager: manager))
        }
    }
}

#Preview("no address") {
    AsyncPreview {
        NavigationStack {
            let manager = WalletManager(preview: "preview_only")

            SendFlowSetAmountScreen(
                id: WalletId()
            )
            .environment(manager)
            .environment(AppManager.shared)
            .environment(SendFlowPresenter(app: AppManager.shared, manager: manager))
        }
    }
}
