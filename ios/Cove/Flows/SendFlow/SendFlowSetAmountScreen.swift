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
private typealias AlertState = SendFlowPresenter.AlertState

struct SendFlowSetAmountScreen: View {
    @Environment(SendFlowPresenter.self) private var presenter
    @Environment(AppManager.self) private var app
    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId
    @State var manager: WalletManager
    @State var address: String = ""
    @State var amount: Amount? = nil

    // private
    @State private var isLoading: Bool = true
    @State private var loadingOpacity: CGFloat = 1

    @FocusState private var _privateFocusField: SendFlowPresenter.FocusField?
    @State private var scrollPosition: ScrollPosition = .init(
        idType: SendFlowPresenter.FocusField.self)

    @State private var scannedCode: TaggedString? = .none

    // fees
    @State private var selectedPresentationDetent: PresentationDetent = .height(440)
    @State private var selectedFeeRate: FeeRateOptionWithTotalFee? = .none
    @State private var feeRateOptions: FeeRateOptionsWithTotalFee? = .none
    @State private var feeRateOptionsBase: FeeRateOptions? = .none

    // text inputs
    @State private var sendAmount: String = "0"
    @State private var sendAmountFiat: String = ""

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    private var network: Network {
        metadata.network
    }

    private var sendAmountSats: Int? {
        let sendAmount = sendAmount.replacingOccurrences(of: ",", with: "")
        guard let amount = Double(sendAmount) else { return .none }

        switch metadata.selectedUnit {
        case .btc:
            return Int(amount * 100_000_000)
        case .sat:
            return Int(amount)
        }
    }

    private var totalFeeString: String {
        guard let totalFee = selectedFeeRate?.totalFee() else { return "---" }

        switch metadata.selectedUnit {
        case .btc:
            return Double(totalFee.asBtc()).btcFmtWithUnit()
        case .sat:
            return totalFee.satsStringWithUnit()
        }
    }

    private var totalSpentInBtc: Double? {
        let sendAmount = sendAmount.replacingOccurrences(of: ",", with: "")

        switch metadata.selectedUnit {
        case .btc:
            let totalSend = Double(sendAmount) ?? 0
            let totalFee = selectedFeeRate?.totalFee().asBtc() ?? 0
            return totalSend + totalFee
        case .sat:
            let totalSend = Int(sendAmount) ?? 0
            let totalFee = Int(selectedFeeRate?.totalFee().asSats() ?? 0)
            let totalSpent = totalSend + totalFee
            return Double(totalSpent) / 100_000_000.0
        }
    }

    private var totalSpent: String {
        guard let totalSpent = totalSpentInBtc else { return "---" }

        switch metadata.selectedUnit {
        case .btc:
            return totalSpent.btcFmtWithUnit()
        case .sat:
            return ThousandsFormatter(totalSpent * 100_000_000).fmtWithUnit()
        }
    }

    private var totalSpentInFiat: String {
        guard let totalSpentInBtc else { return "---" }
        guard let prices = app.prices else { return "---" }

        let fiat = totalSpentInBtc * Double(prices.get())
        return "≈ \(manager.rust.displayFiatAmount(amount: fiat))"
    }

    private var totalSending: String {
        let sendAmount = sendAmount.replacingOccurrences(of: ",", with: "")

        switch metadata.selectedUnit {
        case .btc:
            let totalSend = Double(sendAmount) ?? 0
            return totalSend.btcFmtWithUnit()
        case .sat:
            let totalSend = Int(sendAmount) ?? 0
            return ThousandsFormatter(totalSend).fmtWithUnit()
        }
    }

    private func updateSelectedFeeRate(_ feeRateOptions: FeeRateOptionsWithTotalFee) {
        let selectedFeeRate = {
            switch self.selectedFeeRate?.feeSpeed() {
            case .fast:
                return feeRateOptions.fast()
            case .medium:
                return feeRateOptions.medium()
            case .slow:
                return feeRateOptions.slow()
            case .custom:
                if let custom = feeRateOptions.custom() { return custom }
                Log.debug(
                    "Custom fee rate not found, even tho its selected, keeping current, waiting for update"
                )

                // the fee rate task is probably still resolving, keep selected at custom,
                // and when the task resolves this function will run again and the total fee will be updated
                return self.selectedFeeRate ?? feeRateOptions.medium()
            case nil:
                Log.warn("No fee rate selected, defaulting to medium")
                return feeRateOptions.medium()
            }
        }()

        self.selectedFeeRate = selectedFeeRate
    }

    // MARK: Actions

    private func setAmount(_ amount: Amount) {
        switch metadata.selectedUnit {
        case .btc:
            sendAmount = amount.btcString()
        case .sat:
            sendAmount = amount.satsString()
        }
    }

    private func setMaxSelected(_ selectedFeeRate: FeeRateOptionWithTotalFee?) {
        Log.debug("setMaxSelected")
        let address = try? Address.fromString(address: address, network: network)

        // haven't added address or selected fee rate yet, use a smart default for fee
        guard let address, let selectedFeeRate, let feeRateOptions else {
            Task {
                do {
                    // no address, use own address to buildDrainTx
                    let address = try await manager.firstAddress().address()

                    // rought estimate amount we could send
                    var estimateSend = Int(manager.balance.spendable().asSats()) - 5000
                    estimateSend = max(10_000, estimateSend)

                    let feeRateOptions = try await manager.rust.feeRateOptionsWithTotalFee(
                        feeRateOptions: feeRateOptionsBase,
                        amount: Amount.fromSat(sats: UInt64(estimateSend)),
                        address: address
                    )

                    await MainActor.run {
                        self.feeRateOptions = feeRateOptions
                        updateSelectedFeeRate(feeRateOptions)
                    }

                    let psbt = try await manager.rust.buildDrainTransaction(address: address, fee: feeRateOptions.medium().feeRate())
                    let max = psbt.outputTotalAmount()

                    await MainActor.run {
                        setAmount(max)
                        presenter.maxSelected = max
                    }

                    // update the feeRateOptions with new amount
                    await getFeeRateOptions(address: address, amount: max)
                } catch {
                    Log.error("Unable to set max amount: \(error)")
                }
            }

            return
        }

        Task {
            do {
                let psbt = try await manager.rust.buildDrainTransaction(
                    address: address,
                    fee: selectedFeeRate.feeRate()
                )

                let max = psbt.outputTotalAmount()
                let feeRateOptions = try await manager.rust.feeRateOptionsWithTotalFeeForDrain(
                    feeRateOptions: feeRateOptions, address: address
                )

                updateSelectedFeeRate(feeRateOptions)
                await MainActor.run {
                    self.feeRateOptions = feeRateOptions
                    setAmount(max)
                    presenter.maxSelected = max
                }
            } catch {
                Log.error("Unable to set max amount: \(error)")
            }
        }
    }

    // validate, create final psbt and send to next screen
    private func next() {
        Log.debug("next button pressed")
        guard validate(displayAlert: true) else { return }
        guard let sendAmountSats else {
            return setAlertState(.invalidNumber)
        }

        guard let address = try? Address.fromString(address: address, network: network) else {
            return setAlertState(.invalidAddress(address))
        }

        guard let feeRate = selectedFeeRate else {
            return setAlertState(.unableToGetFeeRate)
        }

        let amount = Amount.fromSat(sats: UInt64(sendAmountSats))

        Task {
            do {
                let confirmDetails = try await manager.rust.confirmTxn(
                    amount: amount,
                    address: address,
                    feeRate: feeRate.feeRate()
                )

                switch metadata.walletType {
                case .xpubOnly, .cold:
                    try manager.rust.saveUnsignedTransaction(details: confirmDetails)
                default: ()
                }

                let route =
                    switch metadata.walletType {
                    case .hot: RouteFactory().sendConfirm(id: id, details: confirmDetails)
                    case .cold: RouteFactory().sendHardwareExport(id: id, details: confirmDetails)
                    case .xpubOnly:
                        RouteFactory().sendHardwareExport(id: id, details: confirmDetails)
                    case .watchOnly: fatalError("can't send from watch only wallet")
                    }

                presenter.focusField = .none
                app.pushRoute(route)
            } catch {
                // error alert is displayed at the top level container, but we can log it here
                Log.error("unable to get confirm details: \(error)")
            }
        }
    }

    // doing it this way prevents an alert popping up when the user just goes back
    private func setAlertState(_ alertState: AlertState) {
        presenter.setAlertState(alertState)
    }

    private func setFormattedAmount(_ amount: String) {
        guard metadata.selectedUnit == .sat else { return }
        guard let amountDouble = Double(amount) else { return }
        let amountInt = Int(round(amountDouble))

        withAnimation {
            sendAmount = ThousandsFormatter(amountInt).fmt()
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
                        EnterAmountView(sendAmount: $sendAmount, sendAmountFiat: $sendAmountFiat)

                        // Address Section
                        VStack {
                            Divider()
                            EnterAddressView(address: $address)
                            Divider()
                        }

                        // Account Section
                        AccountSection

                        if feeRateOptions != nil,
                           selectedFeeRate != nil,
                           Address.isValid(address)
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
                .scrollPosition($scrollPosition, anchor: .top)

                if isLoading {
                    ZStack {
                        Rectangle()
                            .fill(.black)
                            .opacity(loadingOpacity)
                            .ignoresSafeArea()

                        ProgressView().tint(.white)
                            .opacity(loadingOpacity)
                    }
                }
            }
        }
        .padding(.top, 0)
        .onChange(of: _privateFocusField, initial: true) { _, new in
            guard let new else { return }
            presenter.focusField = new
        }
        .onChange(of: presenter.focusField, initial: false, focusFieldChanged)
        .onChange(of: metadata.selectedUnit, initial: false, selectedUnitChanged)
        .onChange(of: sendAmount, initial: true, sendAmountChanged)
        .onChange(of: address, initial: true, addressChanged)
        .onChange(of: scannedCode, initial: false, scannedCodeChanged)
        .onChange(of: selectedFeeRate, initial: true, selectedFeeRateChanged)
        .task {
            guard let feeRateOptions = try? await manager.rust.getFeeOptions() else {
                return
            }

            await MainActor.run {
                feeRateOptionsBase = feeRateOptions
            }
        }
        .task {
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

            // HACK: Bug in SwiftUI where keyboard toolbar is broken
            try? await Task.sleep(for: .milliseconds(700))

            await MainActor.run {
                if sendAmount == "0" || sendAmount == "" {
                    presenter.focusField = .amount
                    return
                }

                if address == "" {
                    presenter.focusField = .address
                    return
                }
            }
        }
        .onAppear {
            if metadata.walletType == .watchOnly {
                app.alertState = .init(.cantSendOnWatchOnlyWallet)
                app.popRoute()
                return
            }

            if sendAmountFiat == "" {
                sendAmountFiat = manager.rust.displayFiatAmount(amount: 0.0)
            }

            // amount
            if let amount {
                presenter.amount = amount

                switch metadata.selectedUnit {
                case .btc: sendAmount = String(amount.btcString())
                case .sat: sendAmount = String(amount.asSats())
                }

                if !validateAmount(displayAlert: true) {
                    presenter.focusField = .amount
                    return
                } else {
                    setFormattedAmount(sendAmount)
                }

                if let prices = app.prices {
                    sendAmountFiat = manager.rust.convertAndDisplayFiat(
                        amount: amount, prices: prices
                    )
                }
            }

            // address
            if address != "" {
                if let address = try? Address.fromString(address: address, network: network) {
                    presenter.address = address
                }

                if !validateAddress(displayAlert: true) {
                    presenter.focusField = .address
                }
            }

            // all valid, scroll to bottom
            if validate() {
                presenter.focusField = .none

                DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) {
                    withAnimation(.easeInOut(duration: 0.4)) {
                        presenter.focusField = .none
                        scrollPosition.scrollTo(edge: .bottom)
                    }
                }
            }
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

    private func validate(displayAlert: Bool = false) -> Bool {
        validateAmount(displayAlert: displayAlert)
            && validateAddress(displayAlert: displayAlert)
    }

    private func validateAddress(
        _ address: String? = nil, displayAlert: Bool = false
    ) -> Bool {
        let address = address ?? self.address
        if address.isEmpty {
            if displayAlert { setAlertState(.emptyAddress) }
            return false
        }

        if case let .failure(error) = Address.checkValid(address) {
            if displayAlert {
                setAlertState(.init(error, address: address))
            }
            return false
        }

        return true
    }

    private func validateAmount(
        _ amount: String? = nil, displayAlert: Bool = false
    ) -> Bool {
        let sendAmountRaw = amount ?? sendAmount
        if displayAlert {
            Log.debug("validating amount: \(sendAmount)")
        }

        let sendAmount = sendAmountRaw.replacingOccurrences(of: ",", with: "")
        guard let amount = Double(sendAmount) else {
            if displayAlert { setAlertState(.invalidNumber) }
            return false
        }

        let balance = Double(manager.balance.spendable().asSats())
        let amountSats = amountSats(amount)

        if amountSats < 5000 {
            if displayAlert { setAlertState(.sendAmountToLow) }
            return false
        }

        if amountSats > balance {
            if displayAlert { setAlertState(.insufficientFunds) }
            return false
        }

        if let selectedFeeRate {
            let totalFeeSats = Double(selectedFeeRate.totalFee().asSats())
            if (amountSats + totalFeeSats) > balance {
                if displayAlert { setAlertState(.insufficientFunds) }
                return false
            }
        }

        return true
    }

    private func amountSats(_ amount: Double) -> Double {
        let maxBtc = 21_000_000
        let maxSats = Double(maxBtc * 100_000_000)

        if amount == 0 {
            return 0
        }

        if metadata.selectedUnit == .sat {
            return min(amount, maxSats)
        }

        return min(amount * 100_000_000, maxSats)
    }

    private func clearSendAmount() {
        Log.debug("clearSendAmount")

        if metadata.fiatOrBtc == .fiat {
            Log.debug("fiat \(sendAmountFiat)")
            sendAmountFiat = app.selectedFiatCurrency.symbol()
            sendAmount = "0"
            return
        }

        if metadata.fiatOrBtc == .btc {
            sendAmount = ""
            sendAmountFiat = manager.rust.displayFiatAmount(amount: 0.0)
            return
        }
    }

    // MARK: OnChange Functions

    // note: maybe this should be moved into `EnterAmountView`
    private func sendAmountChanged(_ oldValue: String, _ newValue: String) {
        Log.debug("sendAmountChanged \(oldValue) -> \(newValue)")
        if feeRateOptions == nil { Task { await getFeeRateOptions() } }

        if metadata.fiatOrBtc == .fiat { return }

        // allow clearing completely
        if newValue == "" {
            return withAnimation { sendAmountFiat = manager.rust.displayFiatAmount(amount: 0.0) }
        }

        // remove leading zeros
        if newValue.hasPrefix("00") {
            sendAmount = String("0")
            return
        }

        if newValue.count == 2, newValue.first == "0", newValue != "0." {
            sendAmount = String(newValue.trimmingPrefix(while: { $0 == "0" }))
            return
        }

        var newValue = newValue

        // no decimals when entering sats
        if metadata.selectedUnit == .sat {
            newValue = newValue.replacingOccurrences(of: ".", with: "")
        }

        if newValue == "." { newValue = "0." }

        let value =
            newValue
                .replacingOccurrences(of: ",", with: "")

        guard let amount = Double(value) else {
            Log.warn("amount not double \(value)")
            sendAmount = oldValue
            return
        }

        let oldValueCleaned =
            oldValue
                .replacingOccurrences(of: ",", with: "")

        // same but formatted, don't do anything
        if oldValueCleaned == value { return }

        // if we had max selected before, but then start entering a different amount
        // cancel max selected
        if let maxSelected = presenter.maxSelected {
            switch metadata.selectedUnit {
            case .sat:
                if amount < Double(maxSelected.asSats()) {
                    presenter.maxSelected = nil
                }

            case .btc:
                if amount < Double(maxSelected.asBtc()) {
                    presenter.maxSelected = nil
                }
            }
        }

        guard let prices = app.prices else {
            Log.warn("unable to get fiat prices")
            app.dispatch(action: .updateFiatPrices)
            sendAmountFiat = "---"
            return
        }

        let amountSats = amountSats(amount)
        presenter.amount = Amount.fromSat(sats: UInt64(amountSats))

        // fiat
        let fiatAmount = (Double(amountSats) / 100_000_000) * Double(prices.get())
        Task { await getFeeRateOptions() }

        withAnimation {
            sendAmountFiat = manager.rust.displayFiatAmount(amount: fiatAmount)
        }

        if metadata.selectedUnit == .sat {
            withAnimation {
                sendAmount = ThousandsFormatter(amountSats).fmt()
            }
        }
    }

    private func selectedUnitChanged(_ oldUnit: Unit, _ newUnit: Unit) {
        let sendAmount = sendAmount.replacingOccurrences(of: ",", with: "")
        guard let amount = Double(sendAmount) else { return }
        if amount == 0 { return }
        if oldUnit == newUnit { return }

        switch newUnit {
        case .btc:
            self.sendAmount = Double(amount / 100_000_000).btcFmt()
        case .sat:
            let sendAmount = Int(amount * 100_000_000)
            if presenter.focusField == .address || presenter.focusField == .none {
                self.sendAmount = ThousandsFormatter(sendAmount).fmt()
            } else {
                self.sendAmount = String(sendAmount)
            }
        }
    }

    // presenter focus field changed
    private func focusFieldChanged(
        _ oldField: FocusField?, _ newField: FocusField?
    ) {
        Log.debug(
            "focusFieldChanged \(String(describing: oldField)) -> \(String(describing: newField))"
        )

        _privateFocusField = newField

        if newField == .none, feeRateOptions == nil {
            let _ = validate(displayAlert: true)
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            withAnimation(.easeInOut(duration: 0.4)) {
                // if keyboard opening directly to amount, dont update scroll position
                if newField == .amount, oldField == .none { return }
                scrollPosition.scrollTo(id: newField)
            }
        }
    }

    private func scannedCodeChanged(_: TaggedString?, _ newValue: TaggedString?) {
        guard let newValue else { return }
        presenter.sheetState = nil

        let addressWithNetwork = try? AddressWithNetwork(address: newValue.item)

        guard let addressWithNetwork else {
            setAlertState(.invalidAddress(newValue.item))
            return
        }

        address = addressWithNetwork.address().string()
        guard validateAddress(address, displayAlert: true) else { return }

        if let amount = addressWithNetwork.amount() {
            setAmount(amount)
            if !validateAmount(displayAlert: true) {
                presenter.focusField = .amount
                return
            }
        }

        if sendAmount == "0" || sendAmount == "" || !validateAmount() {
            return DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                presenter.focusField = .amount
            }
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            presenter.focusField = .none
        }
    }

    private func addressChanged(_: String, _ address: String) {
        if address.isEmpty { return }
        if address.count < 26 || address.count > 62 { return }

        let addressString = address.trimmingCharacters(
            in: .whitespacesAndNewlines)

        guard let address = try? Address.fromString(address: addressString, network: network) else { return }
        guard validateAddress(addressString) else { return }

        presenter.address = address

        let amountSats = max(sendAmountSats ?? 0, 10_000)
        let amount = Amount.fromSat(sats: UInt64(amountSats))

        if presenter.maxSelected != nil {
            Task {
                do {
                    var feeRateOptions = feeRateOptions

                    if feeRateOptions == nil {
                        feeRateOptions = try await manager.rust.feeRateOptionsWithTotalFee(
                            feeRateOptions: feeRateOptionsBase,
                            amount: Amount.fromSat(sats: 10_000),
                            address: address
                        )
                    }

                    let psbt = try await manager.rust.buildDrainTransaction(
                        address: address,
                        fee: feeRateOptions!.medium().feeRate()
                    )

                    let outputAmount = psbt.outputTotalAmount()

                    presenter.maxSelected = outputAmount
                    setAmount(outputAmount)

                } catch {
                    Log.error("Unable to create drain txn: \(error.localizedDescription)")
                }
            }

            return
        }

        // address and amount is valid, dismiss the keyboard
        if validateAmount(), validateAddress(addressString) {
            Log.debug("amount and address valid, dismissing keyboard")
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                presenter.focusField = .none
            }
        }

        Task {
            await getFeeRateOptions(address: address, amount: amount)
        }
    }

    private func selectedFeeRateChanged(
        _: FeeRateOptionWithTotalFee?, _ newFee: FeeRateOptionWithTotalFee?
    ) {
        guard let newFee else { return }
        guard case .custom = newFee.feeSpeed() else { return }
        guard let address = try? Address.fromString(address: address, network: network) else { return }
        guard let sendAmountSats else { return }
        let amount = Amount.fromSat(sats: UInt64(sendAmountSats))

        Task {
            do {
                let psbt = try await manager.rust.buildTransactionWithFeeRate(
                    amount: amount, address: address, feeRate: newFee.feeRate()
                )
                let totalFee = try psbt.fee()
                let feeRate = FeeRateOptionWithTotalFee(
                    feeSpeed: newFee.feeSpeed(), feeRate: newFee.feeRate(), totalFee: totalFee
                )
                guard let feeRateOptions = feeRateOptions?.addCustomFeeRate(feeRate: feeRate) else {
                    return
                }

                await MainActor.run {
                    selectedFeeRate = feeRate
                    self.feeRateOptions = feeRateOptions
                }
            } catch {
                Log.warn("Error building transaction with custom fee rate: \(error)")
            }
        }
    }

    private func getFeeRateOptions(
        address: Address? = nil, amount: Amount? = nil
    ) async {
        let address: Address? = {
            switch address {
            case let .some(address): return address

            case .none:
                let addressString = self.address.trimmingCharacters(
                    in: .whitespacesAndNewlines)
                guard validateAddress(addressString) else { return .none }
                guard
                    let address = try? Address.fromString(
                        address: addressString,
                        network: network
                    )
                else {
                    return .none
                }

                return address
            }
        }()

        guard let address else { return }
        let amount =
            amount ?? Amount.fromSat(sats: UInt64(sendAmountSats ?? 10_000))

        do {
            let feeRateOptions = try await manager.rust.feeRateOptionsWithTotalFee(
                feeRateOptions: feeRateOptionsBase,
                amount: amount,
                address: address
            )

            await MainActor.run {
                self.feeRateOptions = feeRateOptions

                if feeRateOptions.custom() == nil, case .custom = selectedFeeRate?.feeSpeed() {
                    let feeRateOptions = feeRateOptions.addCustomFeeRate(feeRate: selectedFeeRate!)
                    self.feeRateOptions = feeRateOptions
                }

                updateSelectedFeeRate(feeRateOptions)
            }
        } catch {
            Log.error("Unable to get feeRateOptions: \(error)")
        }
    }

    @ViewBuilder
    var AmountKeyboardToolbar: some View {
        HStack {
            Group {
                if address.isEmpty {
                    Button(action: { presenter.focusField = .address }) {
                        Text("Next")
                    }
                } else {
                    Button(action: { presenter.focusField = .none }) {
                        Text("Done")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Spacer()

            Button(action: { setMaxSelected(selectedFeeRate) }) {
                Text("Max")
                    .font(.callout)
            }
            .tint(.primary)
            .buttonStyle(.bordered)

            Button(action: { clearSendAmount() }) {
                Label("Clear", systemImage: "xmark.circle")
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Button(action: { presenter.focusField = .none }) {
                Label("Done", systemImage: "keyboard.chevron.compact.down")
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.primary)
            }
            .buttonStyle(.bordered)
            .tint(.primary)
        }
    }

    @ViewBuilder
    var AddressKeyboardToolbar: some View {
        HStack {
            Group {
                if address.isEmpty || !validateAddress() {
                    Button(action: {
                        address = UIPasteboard.general.string ?? ""
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

            Button(action: { address = "" }) {
                Label("Clear", systemImage: "xmark.circle")
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Button(action: { presenter.focusField = .none }) {
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
        case .amount, .none: AmountKeyboardToolbar
        case .address: AddressKeyboardToolbar
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

                Text(totalSpent)
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
                feeOptions: Binding(get: { feeRateOptions! }, set: { feeRateOptions = $0 }),
                selectedOption: Binding(
                    get: { selectedFeeRate! },
                    set: { newValue in
                        // in maxSelected mode, so adjust with new rate
                        if presenter.maxSelected != nil {
                            setMaxSelected(newValue)
                        }

                        selectedFeeRate = newValue
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

#Preview("with address") {
    AsyncPreview {
        NavigationStack {
            let manager = WalletManager(preview: "preview_only")

            SendFlowSetAmountScreen(
                id: WalletId(),
                manager: manager,
                address: "bc1q08uzlzk9lzq2an7gfn3l4ejglcjgwnud9jgqpc"
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
                id: WalletId(),
                manager: manager,
                address: ""
            )
            .environment(manager)
            .environment(AppManager.shared)
            .environment(SendFlowPresenter(app: AppManager.shared, manager: manager))
        }
    }
}
