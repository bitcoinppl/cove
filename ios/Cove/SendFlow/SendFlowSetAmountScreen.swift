//
//  SendFlowSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

private enum FocusField: Hashable {
    case amount
    case address
}

private enum SheetState: Equatable {
    case qr
    case fee
}

private enum AlertState: Equatable {
    case emptyAddress
    case invalidNumber
    case invalidAddress(String)
    case wrongNetwork(String)
    case noBalance
    case zeroAmount
    case insufficientFunds
    case sendAmountToLow

    init(_ error: AddressError, address: String) {
        switch error {
        case .EmptyAddress: self = .emptyAddress
        case .InvalidAddress: self = .invalidAddress(address)
        case .WrongNetwork: self = .wrongNetwork(address)
        default: self = .invalidAddress(address)
        }
    }
}

// MARK: SendFlowSetAmountScreen

struct SendFlowSetAmountScreen: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId
    @State var model: WalletViewModel
    @State var address: String = ""
    @State var amount: Amount? = nil

    // private
    @State private var isLoading: Bool = true

    @FocusState private var focusField: FocusField?
    @State private var scrollPosition: ScrollPosition = .init(
        idType: FocusField.self)
    @State private var scannedCode: TaggedString? = .none

    // fees
    @State private var txnSize: Int? = nil
    @State private var selectedFeeRate: FeeRateOptionWithTotalFee? = .none
    @State private var feeRateOptions: FeeRateOptionsWithTotalFee? = .none
    @State private var feeRateOptionsBase: FeeRateOptions? = .none

    // alert & sheet
    @State private var sheetState: TaggedItem<SheetState>? = .none
    @State private var alertState: TaggedItem<AlertState>? = .none

    // text inputs
    @State private var sendAmount: String = "0"
    @State private var sendAmountFiat: String = "≈ $0.00 USD"

    // max
    @State private var maxSelected: Amount? = nil

    // shrinking header
    @State private var headerHeight: CGFloat = screenHeight * 0.12
    @State private var scrollOffset: CGFloat = 0

    // back
    @State private var disappearing: Bool = false

    private var metadata: WalletMetadata {
        model.walletMetadata
    }

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { alertState != nil && !disappearing },
            set: { newValue in
                if !newValue {
                    alertState = .none
                }
            }
        )
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
        let sendAmount = self.sendAmount.replacingOccurrences(of: ",", with: "")

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
        return model.fiatAmountToString(fiat)
    }

    private var totalSending: String {
        let sendAmount = self.sendAmount.replacingOccurrences(of: ",", with: "")

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

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER
    private func setMaxSelected(_ selectedFeeRate: FeeRateOptionWithTotalFee) {
        print("setMaxSelected \(selectedFeeRate)")
        Task {
            guard
                let max = try? await model.rust.getMaxSendAmount(
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

    // doing it this way prevents an alert popping up when the user just goes back
    private func setAlertState(_ alertState: AlertState) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            guard !self.disappearing else { return }
            self.alertState = TaggedItem(alertState)
        }
    }

    private func setFormattedAmount(_ amount: String) {
        guard metadata.selectedUnit == .sat else { return }
        guard let amountInt = Int(amount) else { return }
        sendAmount = ThousandsFormatter(amountInt).fmt()
    }



            SendFlowHeaderView(model: model, amount: model.balance.confirmed)
                .frame(height: max(40, headerHeight - scrollOffset))

            // MARK: CONTENT

            ZStack {
                ScrollView {
                    VStack(spacing: 24) {
                        // Set amount, header and text
                        AmountInfoSection

                        // Amount input
                        EnterAmountSection(
                            model: model,
                            sendAmount: $sendAmount,
                            focusField: _focusField,
                            sendAmountFiat: sendAmountFiat
                        )

                        // Address Section
                        EnterAddressSection

                        // Account Section
                        AccountSection

                        if feeRateOptions != nil && selectedFeeRate != nil

                            && Address.isValid(address)
                        {
                            // Total Sending Section
                            TotalSendingSection

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
                .onScrollGeometryChange(for: CGFloat.self) { geometry in
                    geometry.contentOffset.y + geometry.contentInsets.top
                } action: { newOffset, _ in
                    scrollOffset = max(0, newOffset)
                }

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
        .onChange(of: focusField, initial: false, focusFieldChanged)
        .onChange(
            of: metadata.selectedUnit, initial: false, selectedUnitChanged
        )
        .onChange(of: sendAmount, initial: true, sendAmountChanged)
        .onChange(of: address, initial: true, addressChanged)
        .onChange(of: scannedCode, initial: false, scannedCodeChanged)
        .environment(model)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text("Send")
                    .font(.headline)
                    .fontWeight(.semibold)
                    .foregroundColor(.white)
            }
        }
        .task {
            guard let feeRateOptions = try? await model.rust.getFeeOptions()
            else { return }
            await MainActor.run {
                self.feeRateOptionsBase = feeRateOptions
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
                    self.focusField = .address
                    return
                }

                if sendAmount == "0" || sendAmount == "" {
                    self.focusField = .amount
                    return
                }
            }
        }
        .onAppear {
            // if zero balance, show alert and send back
            if model.balance.confirmed.asSats() == 0 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                    withAnimation(.easeInOut(duration: 0.4)) {
                        self.focusField = .none
                    }
                }

                setAlertState(.noBalance)
                return
            }

            // amount
            if let amount = amount {
                switch metadata.selectedUnit {
                case .btc: sendAmount = String(amount.btcString())
                case .sat: sendAmount = String(amount.asSats())
                }

                if !validateAmount(displayAlert: true) {
                    self.focusField = .amount
                } else {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                        setFormattedAmount(sendAmount)
                    }
                }
            }

            // address
            if address != "" {
                if !validateAddress(displayAlert: true) {
                    self.focusField = .address
                }
            }

            if validate() {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                    withAnimation(.easeInOut(duration: 0.4)) {
                        self.focusField = .none
                        scrollPosition.scrollTo(edge: .bottom)
                    }
                }
            }
        }
        .sheet(item: $sheetState, content: SheetContent)
        .onDisappear {
            self.disappearing = true
        }
        .alert(
            alertTitle,
            isPresented: showingAlert,
            presenting: alertState,
            actions: alertButtons,
            message: alertMessage
        )
    }

    // doing it this way prevents an alert popping up when the user just goes back
    private func setAlertState(_ alertState: AlertState) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            guard !self.disappearing else { return }
            self.alertState = TaggedItem(alertState)
        }
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
                setAlertState(AlertState(error, address: address))
            }
            return false
        }

        return true
    }

    private func validateAmount(
        _ amount: String? = nil, displayAlert: Bool = false
    ) -> Bool {
        let sendAmountRaw = amount ?? self.sendAmount
        if displayAlert {
            Log.debug("validating amount: \(sendAmount)")
        }

        let sendAmount = sendAmountRaw.replacingOccurrences(of: ",", with: "")
        guard let amount = Double(sendAmount) else {
            if displayAlert { setAlertState(.invalidNumber) }
            return false
        }

        let balance = Double(model.balance.confirmed.asSats())
        let amountSats = amountSats(amount)

        if amountSats < 10_000 {
            if displayAlert { setAlertState(.sendAmountToLow) }
            return false
        }

        if amountSats > balance {
            if displayAlert { setAlertState(.insufficientFunds) }
            return false
        }

        if let selectedFeeRate = selectedFeeRate {
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

    private func setMaxSelected(_ selectedFeeRate: FeeRateOptionWithTotalFee) {
        print("setMaxSelected \(selectedFeeRate)")
        Task {
            guard
                let max = try? await model.rust.getMaxSendAmount(
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

        let value = newValue.replacingOccurrences(of: ",", with: "")
            .removingLeadingZeros()
        sendAmount = value

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
        if let maxSelected = maxSelected {
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

        sendAmountFiat = model.fiatAmountToString(fiatAmount)

        if oldValue.contains(",") && metadata.selectedUnit == .sat {
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
            if focusField == .address || focusField == .none {
                self.sendAmount = ThousandsFormatter(sendAmount).fmt()
            } else {
                self.sendAmount = String(sendAmount)
            }
        }
    }

    private func focusFieldChanged(
        _ oldField: FocusField?, _ newField: FocusField?
    ) {
        Log.debug(
            "focusFieldChanged \(String(describing: oldField)) -> \(String(describing: newField))"
        )

        if oldField == .amount {
            if !validateAmount(displayAlert: true) { return }
        }

        if oldField == .address {
            if !validateAddress(displayAlert: true) { return }
        }

        let sendAmount = self.sendAmount.replacingOccurrences(of: ",", with: "")
        setFormattedAmount(sendAmount)

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            withAnimation(.easeInOut(duration: 0.4)) {
                if newField == .none && validate() {
                    scrollPosition.scrollTo(edge: .bottom)
                } else {
                    scrollPosition.scrollTo(id: newField)
                }
            }
        }
    }

    private func scannedCodeChanged(_: TaggedString?, _ newValue: TaggedString?)
    {
        guard let newValue = newValue else { return }

        sheetState = nil

        let addressWithNetwork = try? AddressWithNetwork(address: newValue.item)

        guard let addressWithNetwork = addressWithNetwork else {
            setAlertState(.invalidAddress(newValue.item))
            return
        }

        address = addressWithNetwork.address().string()
        guard validateAddress(address, displayAlert: true) else { return }

        if let amount = addressWithNetwork.amount() {
            setAmount(amount)
            if !validateAmount(displayAlert: true) {
                focusField = .amount
                return
            }
        }

        if sendAmount == "0" || sendAmount == "" || !validateAmount() {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                focusField = .amount
            }
            return
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
            focusField = .none
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
                focusField = .none
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

        guard let address = address else { return }
        let amount =
            amount ?? Amount.fromSat(sats: UInt64(sendAmountSats ?? 10000))

        guard
            let feeRateOptions = try? await model.rust
                .feeRateOptionsWithTotalFee(
                    feeRateOptions: feeRateOptionsBase, amount: amount,
                    address: address
                )
        else { return }

        await MainActor.run {
            self.feeRateOptions = feeRateOptions
            if self.selectedFeeRate == nil {
                self.selectedFeeRate = feeRateOptions.medium()
            }
        }
    }

    @ViewBuilder
    var AmountKeyboardToolbar: some View {
        HStack {
            Group {
                if address.isEmpty {
                    Button(action: { focusField = .address }) {
                        Text("Next")
                    }

                } else {
                    Button(action: { focusField = .none }) {
                        Text("Done")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Spacer()

            if let selectedFeeRate = selectedFeeRate {
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

            Button(action: { focusField = .none }) {
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
                if address.isEmpty {
                    Button(action: {
                        address = UIPasteboard.general.string ?? ""
                        if address.isEmpty { return }
                        if !validateAddress() { return }
                        if !validateAmount() {
                            focusField = .amount
                            return
                        }

                        focusField = .none
                        return
                    }) {
                        Text("Paste")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Group {
                if validateAddress() && sendAmount != "" || sendAmount != "0"
                    || !validateAmount()
                {
                    Button(action: { focusField = .amount }) {
                        Text("Next")
                    }
                }
            }
            .buttonStyle(.bordered)
            .tint(.primary)

            Button(action: { sheetState = TaggedItem(.qr) }) {
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

            Button(action: { focusField = .none }) {
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
        switch focusField {
        case .amount, .none: AmountKeyboardToolbar
        case .address: AddressKeyboardToolbar
        }
    }

    @ViewBuilder
    var AmountInfoSection: some View {
        VStack(spacing: 8) {
            HStack {
                Text("Set amount")
                    .font(.title3)
                    .fontWeight(.bold)

                Spacer()
            }
            .id(FocusField.amount)
            .padding(.top, 10)

            HStack {
                Text("How much would you like to send?")
                    .font(.callout)
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
                .font(.callout)
                .foregroundStyle(.secondary)
                .fontWeight(.medium)

            HStack {
                Text(selectedFeeRate?.duration() ?? "2 hours")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Button("Change speed") {
                    self.sheetState = TaggedItem(.fee)
                }
                .font(.caption)
                .foregroundColor(.blue)

                Spacer()

                Text(totalFeeString)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fontWeight(.medium)
            }
        }
        .onTapGesture {
            self.sheetState = TaggedItem(.fee)
        }
    }

    @ViewBuilder
    var EnterAddressSection: some View {
        VStack(spacing: 8) {
            HStack {
                Text("Set address")
                    .font(.headline)
                    .fontWeight(.bold)

                Spacer()
            }
            .id(FocusField.address)
            .padding(.top, 10)

            HStack {
                Text("Where do you want to send to?")
                    .font(.callout)
                    .foregroundStyle(.secondary.opacity(0.80))
                    .fontWeight(.medium)
                Spacer()

                Button(action: { sheetState = TaggedItem(.qr) }) {
                    Image(systemName: "qrcode")
                }
                .foregroundStyle(.secondary)
                .foregroundStyle(.secondary)
            }

            HStack {
                PlaceholderTextEditor(text: $address, placeholder: "bc1q.....")
                    .focused($focusField, equals: .address)
                    .frame(height: 50)
                    .font(.system(size: 16, design: .none))
                    .foregroundStyle(.primary.opacity(0.9))
                    .autocorrectionDisabled(true)
                    .keyboardType(.asciiCapable)
            }
        }
        .padding(.top, 14)
    }

    @ViewBuilder
    var AccountSection: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Image(systemName: "bitcoinsign")
                    .font(.title2)
                    .foregroundColor(.orange)
                    .padding(.trailing, 6)

                VStack(alignment: .leading, spacing: 6) {
                    Text(
                        metadata.masterFingerprint?.asUppercase()
                            ?? "No Fingerprint"
                    )
                    .font(.footnote)
                    .foregroundColor(.secondary)

                    Text(metadata.name)
                        .font(.headline)
                        .fontWeight(.medium)
                }

                Spacer()
            }
            .padding()
            //                        .background(Color(.systemGray6))
            .cornerRadius(12)
        }
    }

    @ViewBuilder
    var TotalSendingSection: some View {
        HStack {
            Text("Total Sending")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fontWeight(.medium)

            Spacer()

            Text(totalSending)
                .font(.callout)
                .foregroundColor(.secondary)
                .fontWeight(.medium)
        }
    }

    @ViewBuilder
    var TotalSpendingSection: some View {
        VStack {
            HStack {
                Text("Total Spending")
                    .font(.title3)
                    .fontWeight(.medium)

                Spacer()

                Text(totalSpent)
                    .multilineTextAlignment(.center)
                    .font(.title3)
                    .fontWeight(.medium)
            }
            .padding(.top, 12)

            HStack {
                Spacer()
                Text(totalSpentInFiat)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            .padding(.top, 1)
        }
    }

    @ViewBuilder
    var NextButtonBottom: some View {
        Button(action: {
            next()
        }) {
            Text("Next")
                .font(.title3)
                .fontWeight(.semibold)
                .frame(maxWidth: .infinity)
                .padding()
                .background(Color.midnightBlue)
                .foregroundColor(.white)
                .cornerRadius(10)
                .disabled(!validate())
        }
        .padding(.vertical)
    }

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .qr:
            QrCodeAddressView(app: _app, scannedCode: $scannedCode)
                .presentationDetents([.large])
        case .fee:
            SendFlowSelectFeeRateView(
                model: model,
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

    // MARK: Alerts

    private var alertTitle: String {
        guard let alertState = alertState else { return "" }

        return {
            switch alertState.item {
            case .emptyAddress, .invalidAddress, .wrongNetwork:
                "Invalid Address"
            case .invalidNumber, .zeroAmount: "Invalid Amount"
            case .insufficientFunds, .noBalance: "Insufficient Funds"
            case .sendAmountToLow: "Send Amount Too Low"
            }
        }()
    }

    @ViewBuilder
    private func alertMessage(alert: TaggedItem<AlertState>) -> some View {
        let text = {
            switch alert.item {
            case .emptyAddress:
                return "Please enter an address"
            case .invalidNumber:
                return "Please enter a valid number for the amout to send"
            case .zeroAmount:
                return
                    "Can't send an empty transaction. Please enter a valid amount"
            case .noBalance:
                return
                    "You do not have any bitcoin in your wallet. Please add some to send a transaction"
            case let .invalidAddress(address):
                return "The address \(address) is invalid"
            case let .wrongNetwork(address):
                return
                    "The address \(address) is on the wrong network. You are on \(metadata.network)"
            case .insufficientFunds:
                return
                    "You do not have enough bitcoin in your wallet to cover the amount plus fees"
            case .sendAmountToLow:
                return "Send amount is too low. Please send atleast 5000 sats"
            }
        }()

        Text(text)
    }

    @ViewBuilder
    private func alertButtons(alert: TaggedItem<AlertState>) -> some View {
        switch alert.item {
        case .emptyAddress, .wrongNetwork, .invalidAddress:
            Button("OK") { focusField = .address }
        case .noBalance:
            Button("Go Back") { app.popRoute() }
        case .invalidNumber, .insufficientFunds, .sendAmountToLow, .zeroAmount:
            Button("OK") { focusField = .amount }
        }
    }
}

private struct EnterAmountSection: View {
    let model: WalletViewModel

    @Binding var sendAmount: String
    @FocusState var focusField: FocusField?
    let sendAmountFiat: String

    // private
    @State private var showingMenu: Bool = false

    var metadata: WalletMetadata { model.walletMetadata }

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                TextField("", text: $sendAmount)
                    .focused($focusField, equals: .amount)
                    .multilineTextAlignment(.center)
                    .font(.system(size: 48, weight: .bold))
                    .keyboardType(
                        metadata.selectedUnit == .btc ? .decimalPad : .numberPad
                    )
                    .offset(
                        x: metadata.selectedUnit == .btc
                            ? screenWidth * 0.10 : screenWidth * 0.11
                    )
                    .padding(.horizontal, 30)
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)

                HStack(spacing: 0) {
                    Button(action: { showingMenu.toggle() }) {
                        Text(model.unit)
                            .padding(.vertical, 10)

                        Image(systemName: "chevron.down")
                            .font(.caption)
                            .fontWeight(.bold)
                            .padding(.top, 2)
                            .padding(.leading, 4)
                    }
                    .foregroundStyle(.primary)
                }
                .popover(isPresented: $showingMenu) {
                    VStack(alignment: .center, spacing: 0) {
                        Button("sats") {
                            model.dispatch(action: .updateUnit(.sat))
                            showingMenu = false
                        }
                        .padding(12)
                        .buttonStyle(.plain)

                        Divider()

                        Button("btc") {
                            model.dispatch(action: .updateUnit(.btc))
                            showingMenu = false
                        }
                        .padding(12)
                        .buttonStyle(.plain)
                    }
                    .padding(.vertical, 8)
                    .padding(.horizontal, 12)
                    .frame(minWidth: 120, maxWidth: 200)
                    .presentationCompactAdaptation(.popover)
                    .foregroundStyle(.primary.opacity(0.8))
                }
            }

            Text(sendAmountFiat)
                .font(.title3)
                .foregroundColor(.secondary)
        }

        .padding(.vertical, 4)
    }
}

#Preview("with address") {
    NavigationStack {
        AsyncPreview {
            SendFlowSetAmountScreen(
                id: WalletId(),
                model: WalletViewModel(preview: "preview_only"),
                address: "bc1q08uzlzk9lzq2an7gfn3l4ejglcjgwnud9jgqpc"
            )
            .environment(MainViewModel())
        }
    }
}

#Preview("no address") {
    NavigationStack {
        AsyncPreview {
            SendFlowSetAmountScreen(
                id: WalletId(),
                model: WalletViewModel(preview: "preview_only"),
                address: ""
            )
            .environment(MainViewModel())
        }
    }
}
