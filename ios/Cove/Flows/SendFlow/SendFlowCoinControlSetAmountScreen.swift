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
    @Environment(SendFlowPresenter.self) private var presenter
    @Environment(AppManager.self) private var app
    @Environment(SendFlowManager.self) private var sendFlowManager
    @Environment(WalletManager.self) private var manager

    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId
    let utxos: [Utxo]

    @State private var scannedCode: TaggedString? = .none

    // fees
    @State private var selectedPresentationDetent: PresentationDetent = .height(440)

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    private var address: Binding<String> {
        sendFlowManager.enteringAddress
    }

    private var network: Network {
        metadata.network
    }

    private var totalSpentInFiat: String {
        sendFlowManager.totalSpentInFiat
    }

    private var totalFeeString: String {
        sendFlowManager.totalFeeString
    }

    private var totalSpentBtc: String {
        sendFlowManager.totalSpentInBtc
    }

    private var totalSending: String {
        sendFlowManager.sendAmountBtc
    }

    private func utxoTotal() -> Amount {
        Amount.fromSat(sats: utxos.reduce(0) { $0 + $1.amount.asSats() })
    }

    private var displayAmount: String {
        sendFlowManager.rust.sendAmountBtc()
    }

    // MARK: Actions

    // validate, create final psbt and send to next screen
    private func next() {
        sendFlowManager.dispatch(action: .finalizeAndGoToNextScreen)
    }

    private func dismissIfValid() {
        if validate(true) {
            presenter.focusField = .none
        }
    }

    // doing it this way prevents an alert popping up when the user just goes back
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

    @ViewBuilder
    var AmountSection: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                Text(displayAmount)
                    .font(.system(size: 48, weight: .bold))
                    .multilineTextAlignment(.center)
                    .keyboardType(.decimalPad)
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)
                    .scrollDisabled(true)
                    .offset(x: offset)
                    .padding(.horizontal, 30)
                    .frame(height: UIFont.boldSystemFont(ofSize: 48).lineHeight)

                HStack(spacing: 0) {
                    Menu {
                        VStack(alignment: .center, spacing: 0) {
                            Button(action: {
                                manager.dispatch(action: .updateUnit(.sat))
                            }) {
                                Text("sats")
                                    .frame(maxWidth: .infinity)
                                    .padding(12)
                                    .background(Color.clear)
                            }
                            .buttonStyle(.plain)
                            .contentShape(Rectangle())

                            Button(action: {
                                manager.dispatch(action: .updateUnit(.btc))
                            }) {
                                Text("btc")
                                    .frame(maxWidth: .infinity)
                                    .padding(12)
                                    .background(Color.clear)
                            }
                            .buttonStyle(.plain)
                            .contentShape(Rectangle())
                        }
                        .foregroundStyle(.primary.opacity(0.8))
                        .contentShape(Rectangle())
                    } label: {
                        HStack(spacing: 2) {
                            Text(manager.unit)
                                .padding(.vertical, 10)
                                .padding(.horizontal, 10)
                                .fixedSize(horizontal: true, vertical: true)

                            Image(systemName: "chevron.down")
                                .font(.caption)
                                .fontWeight(.bold)
                                .padding(.top, 2)
                        }
                        .offset(y: -2)
                    }
                    .foregroundStyle(.primary)
                }
            }

            Text(sendFlowManager.rust.sendAmountFiat())
                .contentTransition(.numericText())
                .font(.subheadline)
                .foregroundColor(.secondary)
        }
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
                        AmountInfoSection

                        // Amount input
                        AmountSection

                        // Address Section
                        VStack {
                            Divider()
                            EnterAddressView(address: sendFlowManager.enteringAddress)
                            Divider()
                        }

                        // Account Section
                        AccountSection

                        if sendFlowManager.feeRateOptions != nil,
                            sendFlowManager.address != nil
                        {
                            // Network Fee Section
                            NetworkFeeSection

                            // Total Spending Section
                            TotalSpendingSection

                            // Next Button
                            NextButtonBottom
                        }
                    }

                    .toolbar {
                        ToolbarItemGroup(placement: .keyboard) {
                            ToolBarView
                        }
                    }
                }
                .padding(.horizontal)
                .frame(maxWidth: .infinity)
                .background(colorScheme == .light ? .white : .black)
                .scrollIndicators(.hidden)
            }
        }
        .padding(.top, 0)
        .onChange(of: presenter.focusField, initial: true, focusFieldChanged)
        .onChange(of: scannedCode, initial: false, scannedCodeChanged)
        .onChange(of: metadata.selectedUnit, initial: false) { oldUnit, newUnit in
            sendFlowManager.dispatch(.notifySelectedUnitedChanged(old: oldUnit, new: newUnit))
        }
        .onChange(of: metadata.fiatOrBtc, initial: false) { old, new in
            sendFlowManager.dispatch(.notifyBtcOrFiatChanged(old: old, new: new))
        }
        .onChange(of: app.prices, initial: true) { _, newPrices in
            guard let prices = newPrices else { return }
            sendFlowManager.dispatch(.notifyPricesChanged(prices))
        }

        .onAppear {
            presenter.focusField = .address

            if metadata.walletType == .watchOnly {
                app.alertState = .init(.cantSendOnWatchOnlyWallet)
                app.popRoute()
                return
            }

            sendFlowManager.setAmount(utxoTotal())
        }
        .sheet(item: presenter.sheetStateBinding, content: SheetContent)
        .alert(
            presenter.alertTitle,
            isPresented: presenter.showingAlert,
            presenting: presenter.alertState,
            actions: presenter.alertButtons,
            message: presenter.alertMessage
        )
    }

    private func validate(_ displayAlert: Bool = false) -> Bool {
        validateAmount(displayAlert: displayAlert)
            && validateAddress(displayAlert: displayAlert)
    }

    private func validateAddress(_: String? = nil, displayAlert: Bool = false) -> Bool {
        sendFlowManager.rust.validateAddress(displayAlert: displayAlert)
    }

    private func validateAmount(_: String? = nil, displayAlert: Bool = false) -> Bool {
        sendFlowManager.rust.validateAmount(displayAlert: displayAlert)
    }

    // MARK: OnChange Functions

    private func selectedUnitChanged(oldUnit: Unit, newUnit: Unit) {
        Log.debug("selectedUnitChanged \(oldUnit) -> \(newUnit)")
        sendFlowManager.dispatch(action: .notifySelectedUnitedChanged(old: oldUnit, new: newUnit))
    }

    // presenter focus field changed
    private func focusFieldChanged(_ oldField: FocusField?, _ newField: FocusField?) {
        Log.debug(
            "focusFieldChanged \(String(describing: oldField)) -> \(String(describing: newField))")

        sendFlowManager.dispatch(action: .notifyFocusFieldChanged(old: oldField, new: newField))
    }

    private func clearAddress() {
        Log.debug("clearAddress")
        sendFlowManager.dispatch(action: .clearAddress)
    }

    private func scannedCodeChanged(old: TaggedString?, newValue: TaggedString?) {
        Log.debug(
            "scannedCodeChanged \(String(describing: old)) -> \(String(describing: newValue))")
        guard let newValue else { return }
        presenter.sheetState = nil
        sendFlowManager.dispatch(
            action: .notifyScanCodeChanged(old: old?.item ?? "", new: newValue.item))
    }

    @ViewBuilder
    var AddressKeyboardToolbar: some View {
        HStack {
            Group {
                if address.wrappedValue.isEmpty || !validateAddress() {
                    Button(action: {
                        let address = UIPasteboard.general.string ?? ""
                        sendFlowManager.dispatch(action: .changeEnteringAddress(address))
                        if address.isEmpty { return }
                        if !validateAddress() { return }
                        if !validateAmount() {
                            presenter.focusField = .amount
                            return
                        }

                        presenter.focusField = .none
                    }) {
                        Text("Paste")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Group {
                if validateAddress(), !validateAmount() {
                    Button(action: { presenter.focusField = .amount }) {
                        Text("Next")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Button(action: { presenter.sheetState = TaggedItem(.qr) }) {
                Label("QR", systemImage: "qrcode")
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Spacer()

            Button(action: { clearAddress() }) {
                Label("Clear", systemImage: "xmark.circle")
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Button(action: dismissIfValid) {
                Label("Done", systemImage: "keyboard.chevron.compact.down")
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.bordered)
            .tint(.primary)
        }
    }

    @ViewBuilder
    var ToolBarView: some View {
        switch presenter.focusField {
        case .address: AddressKeyboardToolbar
        case .amount, .none: EmptyView()
        }
    }

    @ViewBuilder
    var AmountInfoSection: some View {
        VStack(spacing: 8) {
            HStack {
                Text("Enter amount")
                    .font(.headline)
                    .fontWeight(.bold)

                Spacer()
            }
            .id(FocusField.amount)

            HStack {
                Text("How much would you like to send?")
                    .font(.footnote)
                    .foregroundStyle(.secondary.opacity(0.80))
                    .fontWeight(.medium)
                Spacer()
            }
        }
        .padding(.top)
    }

    @ViewBuilder
    var NetworkFeeSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Network Fee")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fontWeight(.medium)

            HStack {
                Text(selectedFeeRate?.duration() ?? "2 hours")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                Button("Change speed") {
                    selectedPresentationDetent =
                        if feeRateOptions?.custom() == nil { .height(440) } else { .height(550) }
                    presenter.sheetState = TaggedItem(.fee)
                }
                .font(.caption2)
                .foregroundColor(.blue)

                Spacer()

                Text(totalFeeString)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fontWeight(.medium)
            }
        }
        .onTapGesture {
            presenter.sheetState = TaggedItem(.fee)
        }
    }

    @ViewBuilder
    var AccountSection: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Account")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fontWeight(.medium)

                Spacer()

                if metadata.walletType == .hot {
                    Image(systemName: "bitcoinsign")
                        .font(.title2)
                        .foregroundColor(.orange)
                        .padding(.trailing, 6)
                }

                if case .cold = metadata.walletType {
                    BitcoinShieldIcon(width: 24, color: .orange)
                }

                VStack(alignment: .leading, spacing: 6) {
                    Text(metadata.identOrFingerprint())
                        .font(.footnote)
                        .foregroundColor(.secondary)
                        .fontWeight(.medium)

                    Text(metadata.name)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
            }
        }
    }

    @ViewBuilder
    var TotalSpendingSection: some View {
        VStack {
            HStack {
                Text("Total Spending")
                    .font(.footnote)
                    .fontWeight(.semibold)

                Spacer()

                Text(totalSpentBtc)
                    .multilineTextAlignment(.center)
                    .font(.footnote)
                    .fontWeight(.semibold)
            }

            HStack {
                Spacer()
                Text(totalSpentInFiat)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            .padding(.top, 1)
        }
    }

    @ViewBuilder
    var NextButtonBottom: some View {
        Button(action: next) {
            Text("Next")
                .font(.footnote)
                .fontWeight(.semibold)
                .frame(maxWidth: .infinity)
                .padding()
                .background(Color.midnightBtn)
                .foregroundColor(.white)
                .cornerRadius(10)
                .disabled(!validate())
        }
        .padding(.vertical, 10)
    }

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .qr:
            QrCodeAddressView(app: _app, scannedCode: $scannedCode)
                .presentationDetents([.large])
        case .fee:
            SendFlowSelectFeeRateView(
                manager: manager,
                feeOptions: Binding(
                    get: { sendFlowManager.feeRateOptions! },
                    set: { sendFlowManager.dispatch(action: .changeFeeRateOptions($0)) }
                ),
                selectedOption: Binding(
                    get: {
                        guard let selectedFeeRate = sendFlowManager.selectedFeeRate else {
                            // Default to medium if nothing selected
                            if let options = sendFlowManager.feeRateOptions {
                                return options.medium()
                            }
                            return FeeRateOptionsWithTotalFee.previewNew().medium()
                        }
                        return selectedFeeRate
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

#Preview {
    AsyncPreview {
        NavigationStack {
            let manager = WalletManager(preview: "preview_only")
            let presenter = SendFlowPresenter(app: AppManager.shared, manager: manager)
            let sendFlowManager = SendFlowManager(
                manager.rust.newSendFlowManager(),
                presenter: presenter
            )

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
