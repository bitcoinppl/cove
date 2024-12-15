//
//  SendFlowSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

// MARK: SendFlowSetAmountScreen

private typealias FocusField = SendFlowSetAmountPresenter.FocusField
private typealias SheetState = SendFlowSetAmountPresenter.SheetState
private typealias AlertState = SendFlowSetAmountPresenter.AlertState

struct SendFlowSetAmountScreen: View {
    @Environment(SendFlowSetAmountPresenter.self) private var presenter
    @Environment(AppManager.self) private var app
    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId
    @State var manager: WalletManager
    @State var address: String = ""
    @State var amount: Amount? = nil

    // private
    @State private var isLoading: Bool = true

    @FocusState private var _privateFocusField: SendFlowSetAmountPresenter.FocusField?
    @State private var scrollPosition: ScrollPosition = .init(
        idType: SendFlowSetAmountPresenter.FocusField.self)

    @State private var scannedCode: TaggedString? = .none

    // fees
    @State private var txnSize: Int? = nil
    @State private var selectedFeeRate: FeeRateOptionWithTotalFee? = .none
    @State private var feeRateOptions: FeeRateOptionsWithTotalFee? = .none
    @State private var feeRateOptionsBase: FeeRateOptions? = .none

    // text inputs
    @State private var sendAmount: String = "0"
    @State private var sendAmountFiat: String = "≈ $0.00 USD"

    // max
    @State private var maxSelected: Amount? = nil

    private var metadata: WalletMetadata {
        manager.walletMetadata
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
        guard let usd = app.prices?.usd else { return "---" }

        let fiat = totalSpentInBtc * Double(usd)
        return manager.fiatAmountToString(fiat)
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

    // MARK: Actions

    private func setAmount(_ amount: Amount) {
        switch metadata.selectedUnit {
        case .btc:
            sendAmount = amount.btcString()
        case .sat:
            sendAmount = amount.satsString()
        }
    }

    private func setMaxSelected(_ selectedFeeRate: FeeRateOptionWithTotalFee) {
        Task {
            guard
                let max = try? await manager.rust.getMaxSendAmount(
                    fee: selectedFeeRate)
            else {
                return Log.error("unable to get max send amount")
            }

            await MainActor.run {
                setAmount(max)
                maxSelected = max
            }
        }
    }

    // validate, create final psbt and send to next screen
    private func next() {
        guard validate(displayAlert: true) else { return }
        guard let sendAmountSats else {
            return setAlertState(.invalidNumber)
        }

        guard let address = try? Address.fromString(address: address) else {
            return setAlertState(.invalidAddress(address))
        }

        guard let feeRate = selectedFeeRate else {
            return setAlertState(.unableToGetFeeRate)
        }

        let amount = Amount.fromSat(sats: UInt64(sendAmountSats))

        Task {
            do {
                let confirmDetails = try await manager.rust.getConfirmDetails(
                    amount: amount,
                    address: address,
                    feeRate: feeRate.feeRate()
                )

                if case .cold = metadata.walletType {
                    try? manager.rust.saveUnsignedTransaction(details: confirmDetails)
                }

                let route =
                    switch metadata.walletType {
                    case .hot: RouteFactory().sendConfirm(id: id, details: confirmDetails)
                    case .cold: RouteFactory().sendHardwareExport(id: id, details: confirmDetails)
                    }

                app.pushRoute(route)
            } catch {
                Log.error("unable to get confirm details: \(error)")
                setAlertState(.unableToBuildTxn(error.localizedDescription))
            }
        }
    }

    // doing it this way prevents an alert popping up when the user just goes back
    private func setAlertState(_ alertState: AlertState) {
        presenter.setAlertState(alertState)
    }

    private func setFormattedAmount(_ amount: String) {
        guard metadata.selectedUnit == .sat else { return }
        guard let amountInt = Int(amount) else { return }
        sendAmount = ThousandsFormatter(amountInt).fmt()
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(manager: manager, amount: manager.balance.confirmed)

            // MARK: CONTENT

            ZStack {
                ScrollView {
                    VStack(spacing: 24) {
                        // Set amount, header and text
                        AmountInfoSection

                        // Amount input
                        EnterAmountView(sendAmount: $sendAmount, sendAmountFiat: sendAmountFiat)

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
                        Color.primary.ignoresSafeArea(.all).opacity(
                            isLoading ? 1 : 0)
                        ProgressView().tint(.white)
                    }
                }
            }
        }
        .padding(.top, 0)
        .onChange(of: _privateFocusField, initial: true) { _, new in
            presenter.focusField = new
        }
        .onChange(of: presenter.focusField, initial: false, focusFieldChanged)
        .onChange(
            of: metadata.selectedUnit, initial: false, selectedUnitChanged
        )
        .onChange(of: sendAmount, initial: true, sendAmountChanged)
        .onChange(of: address, initial: true, addressChanged)
        .onChange(of: scannedCode, initial: false, scannedCodeChanged)
        .task {
            guard let feeRateOptions = try? await manager.rust.getFeeOptions()
            else { return }
            await MainActor.run {
                feeRateOptionsBase = feeRateOptions
            }
        }
        .task {
            Task {
                try? await Task.sleep(for: .milliseconds(600))
                await MainActor.run {
                    withAnimation {
                        isLoading = false
                    }
                }
            }

            // HACK: Bug in SwiftUI where keyboard toolbar is broken
            try? await Task.sleep(for: .milliseconds(700))

            await MainActor.run {
                if address == "" {
                    presenter.focusField = .address
                    return
                }

                if sendAmount == "0" || sendAmount == "" {
                    presenter.focusField = .amount
                    return
                }
            }
        }
        .onAppear {
            // amount
            if let amount {
                switch metadata.selectedUnit {
                case .btc: sendAmount = String(amount.btcString())
                case .sat: sendAmount = String(amount.asSats())
                }

                if !validateAmount(displayAlert: true) {
                    presenter.focusField = .amount
                } else {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                        setFormattedAmount(sendAmount)
                    }
                }
            }

            // address
            if address != "" {
                if !validateAddress(displayAlert: true) {
                    presenter.focusField = .address
                }
            }

            // all valid, scroll to bottom
            if validate() {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
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

        let balance = Double(manager.balance.confirmed.asSats())
        let amountSats = amountSats(amount)

        if amountSats < 10000 {
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
        if amount == 0 {
            return 0
        }

        if metadata.selectedUnit == .sat {
            return amount
        }

        return amount * 100_000_000
    }

    // MARK: OnChange Functions

    private func sendAmountChanged(_ oldValue: String, _ newValue: String) {
        Log.debug("sendAmountChanged \(oldValue) -> \(newValue)")

        if feeRateOptions == nil {
            Task { await getFeeRateOptions() }
        }

        // allow clearing completely
        if newValue == "" {
            sendAmountFiat = "≈ $0.00 USD"
            return
        }

        let value =
            newValue
                .replacingOccurrences(of: ",", with: "")
                .removingLeadingZeros()

        if presenter.focusField == .amount {
            sendAmount = value
        }

        guard let amount = Double(value) else {
            sendAmount = oldValue
            return
        }

        let oldValueCleaned =
            oldValue
                .replacingOccurrences(of: ",", with: "")
                .removingLeadingZeros()

        if oldValueCleaned == value { return }

        // if we had max selected before, but then start entering a different amount
        // cancel max selected
        if let maxSelected {
            switch metadata.selectedUnit {
            case .sat:
                if amount < Double(maxSelected.asSats()) {
                    self.maxSelected = nil
                }
            case .btc:
                if amount < Double(maxSelected.asBtc()) {
                    self.maxSelected = nil
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
        let fiatAmount = (amountSats / 100_000_000) * Double(prices.usd)

        if feeRateOptions == nil {
            Task { await getFeeRateOptions() }
        }

        sendAmountFiat = manager.fiatAmountToString(fiatAmount)

        if oldValue.contains(","), metadata.selectedUnit == .sat {
            setFormattedAmount(String(amountSats))
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

        if oldField == .amount {
            if !validateAmount(displayAlert: true) { return }
        }

        if oldField == .address {
            if !validateAddress(displayAlert: true) { return }
        }

        let sendAmount = sendAmount.replacingOccurrences(of: ",", with: "")
        if newField == .amount {
            self.sendAmount = sendAmount
        } else {
            setFormattedAmount(sendAmount)
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            withAnimation(.easeInOut(duration: 0.4)) {
                if newField == .none, validate() {
                    scrollPosition.scrollTo(edge: .bottom)
                } else {
                    scrollPosition.scrollTo(id: newField)
                }
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
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                presenter.focusField = .amount
            }
            return
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
        guard let address = try? Address.fromString(address: addressString)
        else { return }
        guard validateAddress(addressString) else { return }

        let amountSats = max(sendAmountSats ?? 0, 10000)
        let amount = Amount.fromSat(sats: UInt64(amountSats))

        // address and amount is valid, dismiss the keyboard
        if validateAmount() {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                presenter.focusField = .none
            }
        }

        Task {
            await getFeeRateOptions(address: address, amount: amount)
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
                        address: addressString)
                else {
                    return .none
                }

                return address
            }
        }()

        guard let address else { return }
        let amount =
            amount ?? Amount.fromSat(sats: UInt64(sendAmountSats ?? 10000))

        guard
            let feeRateOptions = try? await manager.rust
            .feeRateOptionsWithTotalFee(
                feeRateOptions: feeRateOptionsBase, amount: amount,
                address: address
            )
        else { return }

        await MainActor.run {
            self.feeRateOptions = feeRateOptions
            if selectedFeeRate == nil {
                selectedFeeRate = feeRateOptions.medium()
            }
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

            if let selectedFeeRate {
                Button(action: { setMaxSelected(selectedFeeRate) }) {
                    Text("Max")
                        .font(.callout)
                }
                .tint(.primary)
                .buttonStyle(.bordered)
            }

            Button(action: { sendAmount = "" }) {
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
                if sendAmount != "" || sendAmount != "0"
                    || !validateAmount(), validateAddress()
                {
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

                if metadata.walletType == .cold {
                    BitcoinShieldIcon(width: 24, color: .orange)
                }

                VStack(alignment: .leading, spacing: 6) {
                    Text(
                        metadata.masterFingerprint?.asUppercase()
                            ?? "No Fingerprint"
                    )
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
                feeOptions: feeRateOptions!,
                selectedOption: Binding(
                    get: { selectedFeeRate! },
                    set: { newValue in
                        // in maxSelected mode, so adjust with new rate
                        if maxSelected != nil {
                            setMaxSelected(newValue)
                        }

                        selectedFeeRate = newValue
                    }
                )
            )
            .presentationDetents([.height(400)])
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
            .environment(AppManager())
            .environment(SendFlowSetAmountPresenter(app: AppManager(), manager: manager))
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
            .environment(AppManager())
            .environment(SendFlowSetAmountPresenter(app: AppManager(), manager: manager))
        }
    }
}
