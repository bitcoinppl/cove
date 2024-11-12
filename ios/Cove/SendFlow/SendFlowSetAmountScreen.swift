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

    // private
    @FocusState private var focusField: FocusField?
    @State private var scrollPosition: ScrollPosition = .init(idType: FocusField.self)
    @State private var scannedCode: TaggedString? = .none

    // fees
    @State private var txnSize: Int? = nil
    @State private var totalFee: Int? = nil
    @State private var selectedFeeRate: FeeRateOption? = .none
    @State private var feeRateOptions: FeeRateOptions? = .none

    // alert & sheet
    @State private var sheetState: TaggedItem<SheetState>? = .none
    @State private var alertState: TaggedItem<AlertState>? = .none

    // text inputs
    @State private var sendAmount: String = "0"
    @State private var sendAmountFiat: String = "≈ $0.00"

    private var metadata: WalletMetadata {
        model.walletMetadata
    }

    private var formatter: NumberFormatter {
        let f = NumberFormatter()
        f.numberStyle = .currency
        f.minimumFractionDigits = 2
        f.maximumFractionDigits = 2
        return f
    }

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { alertState != nil },
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
        if let totalFee = totalFee {
            return "\(String(totalFee)) sats"
        }

        return "---"
    }

    var body: some View {
        VStack(spacing: 0) {
            // MARK: HEADER

            SendFlowHeaderView(model: model, amount: model.balance.confirmed)

            // MARK: CONTENT

            ScrollView {
                VStack(spacing: 24) {
                    // Set amount, header and text
                    AmountInfoSection

                    // Amount input
                    EnterAmountSection

                    // Address Section
                    EnterAddressSection

                    // Account Section
                    AccountSection

                    if feeRateOptions != nil && selectedFeeRate != nil && Address.isValid(address) {
                        // Network Fee Section
                        NetworkFeeSection

                        // Total Section
                        TotalSection

                        Spacer()

                        // Next Button
                        NextButtonBottom
                    }
                }
            }
            .padding(.horizontal)
            .frame(maxWidth: .infinity)
            .background(colorScheme == .light ? .white : .black)
            .scrollIndicators(.hidden)
            .scrollPosition($scrollPosition, anchor: .top)
        }
        .padding(.top, 0)
        .navigationTitle("Send")
        .navigationBarTitleDisplayMode(.inline)
        .onChange(of: focusField, initial: false, focusFieldChanged)
        .onChange(of: metadata.selectedUnit, initial: false, selectedUnitChanged)
        .onChange(of: sendAmount, initial: false, sendAmountChanged)
        .onChange(of: scannedCode, initial: false, scannedCodeChanged)
        .onChange(of: address, initial: true, addressChanged)
        .onChange(of: selectedFeeRate, initial: false, selectedFeeRateChanged)
        .onAppear {
            print("ON APPEAR", sendAmount, address)
            DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(100)) {
                if sendAmount == "0" || sendAmount == "" {
                    focusField = .amount
                    return
                }

                if address == "" {
                    focusField = .address
                    return
                }
            }
        }
        .sheet(item: $sheetState, content: SheetContent)
        .task {
            guard let feeRateOptions = try? await model.rust.feeRateOptions() else { return }
            await MainActor.run {
                self.feeRateOptions = feeRateOptions
                if selectedFeeRate == nil {
                    selectedFeeRate = feeRateOptions.medium()
                }
            }
        }
        .alert(
            alertTitle,
            isPresented: showingAlert,
            presenting: alertState,
            actions: alertButtons,
            message: alertMessage
        )
        .toolbar {
            ToolbarItemGroup(placement: .keyboard) {
                ToolBarView
            }
        }
    }

    private func validate(displayAlert: Bool = false) -> Bool {
        validateAmount(displayAlert: displayAlert) && validateAddress(displayAlert: displayAlert)
    }

    private func validateAddress(_ address: String? = nil, displayAlert: Bool = false) -> Bool {
        let address = address ?? self.address
        if address.isEmpty {
            if displayAlert { alertState = TaggedItem(.emptyAddress) }
            return false
        }

        if case let .failure(error) = Address.checkValid(address) {
            if displayAlert { alertState = TaggedItem(AlertState(error, address: address)) }
            return false
        }

        return true
    }

    private func validateAmount(_ amount: String? = nil, displayAlert: Bool = false) -> Bool {
        let sendAmount = amount ?? self.sendAmount
        guard let amount = Double(sendAmount) else {
            if displayAlert { alertState = TaggedItem(.invalidNumber) }
            return false
        }

        let balance = Double(model.balance.confirmed.asSats())
        let amountSats = amountSats(amount)

        // TODO: check if amount + fees is less than balance
        if amountSats > balance {
            if displayAlert { alertState = TaggedItem(.insufficientFunds) }
            return false
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

    private func sendAmountChanged(_ oldValue: String, _ value: String) {
        // allow clearing completely
        if value == "" {
            sendAmountFiat = "≈ $0.00"
            return
        }

        if metadata.selectedUnit == .sat && value.contains(",") { return }

        let value = value.removingLeadingZeros()
        sendAmount = value

        guard let amount = Double(value) else {
            sendAmount = oldValue
            return
        }

        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            sendAmountFiat = "---"
            return
        }

        let amountSats = amountSats(amount)
        let fiatAmount = (amountSats / 100_000_000) * Double(prices.usd)

        sendAmountFiat = "≈ \(formatter.string(from: NSNumber(value: fiatAmount)) ?? "$0.00")"
    }

    private func selectedUnitChanged(_ oldUnit: Unit, _ newUnit: Unit) {
        let sendAmount = sendAmount.replacingOccurrences(of: ",", with: "")
        guard let amount = Double(sendAmount) else { return }
        if amount == 0 { return }
        if oldUnit == newUnit { return }

        switch newUnit {
        case .btc:
            self.sendAmount = String(amount / 100_000_000)
        case .sat:
            let sendAmount = Int(amount * 100_000_000)
            if focusField == .address || focusField == .none {
                self.sendAmount = ThousandsFormatter(sendAmount).fmt()
            } else {
                self.sendAmount = String(sendAmount)
            }
        }
    }

    private func focusFieldChanged(_: FocusField?, _ newField: FocusField?) {
        let sendAmount = self.sendAmount.replacingOccurrences(of: ",", with: "")

        DispatchQueue.main.async {
            if let sendAmountInt = Int(sendAmount), metadata.selectedUnit == .sat {
                switch newField {
                case .amount: self.sendAmount = String(sendAmountInt)
                case .address, .none:
                    self.sendAmount = ThousandsFormatter(sendAmountInt).fmt()
                }
            }
        }

        DispatchQueue.main.async {
            withAnimation(.easeInOut(duration: 0.3)) {
                scrollPosition.scrollTo(id: newField)
            }
        }
    }

    private func scannedCodeChanged(_: TaggedString?, _ newValue: TaggedString?) {
        guard let newValue = newValue else { return }
        sheetState = nil

        if validateAddress(newValue.item, displayAlert: true) {
            address = newValue.item
            focusField = .none
        } else {
            address = ""
        }
    }

    private func addressChanged(_: String, _ address: String) {
        if address.isEmpty { return }
        if address.count < 26 || address.count > 62 { return }

        let addressString = address.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let address = try? Address.fromString(address: addressString) else { return }
        guard validateAddress(addressString) else { return }

        updateTotalFee(selectedFeeRate: selectedFeeRate, address: address)
    }

    private func selectedFeeRateChanged(_: FeeRateOption?, _ selectedFeeRate: FeeRateOption?) {
        updateTotalFee(selectedFeeRate: selectedFeeRate)
    }

    private func updateTotalFee(selectedFeeRate: FeeRateOption?, address: Address? = nil) {
        guard let selectedFeeRate = selectedFeeRate else { return }

        let address: Address? = {
            switch address {
            case let .some(address): return address
            case .none:
                let addressString = self.address.trimmingCharacters(in: .whitespacesAndNewlines)
                guard validateAddress(addressString) else { return nil }
                guard let address = try? Address.fromString(address: addressString) else {
                    return nil
                }

                return address
            }
        }()

        guard let address = address else { return }
        let amountSats = max(sendAmountSats ?? 0, 10000)
        let amount = Amount.fromSat(sats: UInt64(amountSats))

        let psbt = try? model.rust.buildTransactionWithFeeRate(
            amount: amount, address: address,
            feeRate: selectedFeeRate.rate()
        )

        if let psbt = psbt {
            do {
                let fee = try psbt.fee()
                let totalFee = Int(fee.asSats())

                self.totalFee = totalFee
            } catch {
                Log.warn("Failed to get PSBT: \(error)")
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
            .font(.callout)
            .padding(.vertical, 7)
            .padding(.horizontal, 12)
            .background(.midnightBlue.opacity(0.2))
            .cornerRadius(7)
            .foregroundStyle(.midnightBlue)
            .buttonStyle(.plain)

            Spacer()

            Button(action: {
                // TODO: add  max
            }) {
                Text("Max")
                    .font(.callout)
            }
            .tint(.primary)
            .buttonStyle(.bordered)

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
                        if !address.isEmpty {
                            focusField = .none
                        }
                    }) {
                        Text("Paste")
                    }
                } else {
                    Button(action: { focusField = .none }) {
                        Text("Done")
                    }
                }
            }
            .padding(.vertical, 7)
            .padding(.horizontal, 12)
            .background(.midnightBlue.opacity(0.2))
            .foregroundStyle(.midnightBlue)
            .cornerRadius(7)
            .buttonStyle(.plain)
            .font(.callout)

            Button(action: { sheetState = TaggedItem(.qr) }) {
                Label("QR", systemImage: "qrcode")
            }
            .padding(.vertical, 5.25)
            .padding(.horizontal, 12)
            .background(.midnightBlue.opacity(0.2))
            .foregroundStyle(.midnightBlue)
            .cornerRadius(7)
            .buttonStyle(.plain)
            .font(.callout)

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
    var EnterAmountSection: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                TextField("", text: $sendAmount)
                    .focused($focusField, equals: .amount)
                    .multilineTextAlignment(.center)
                    .font(.system(size: 48, weight: .bold))
                    .keyboardType(metadata.selectedUnit == .btc ? .decimalPad : .numberPad)
                    .offset(x: screenWidth * 0.06)
                    .padding(.horizontal, 30)
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)

                Text(model.unit)
                    .padding(.vertical, 10)
                    .contentShape(
                        .contextMenuPreview, RoundedRectangle(cornerRadius: 8).inset(by: -5)
                    )
                    .contextMenu(
                        menuItems: {
                            Button {
                                model.dispatch(action: .updateUnit(.btc))
                            } label: {
                                Text("btc")
                            }

                            Button {
                                model.dispatch(action: .updateUnit(.sat))
                            } label: {
                                Text("sats")
                            }
                        },
                        preview: {
                            Text(model.unit)
                                .padding(12)
                                .clipShape(RoundedRectangle(cornerRadius: 8))
                        }
                    )
            }

            Text(sendAmountFiat)
                .font(.title3)
                .foregroundColor(.secondary)
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder
    var NetworkFeeSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Network Fee")
                .font(.headline)
                .foregroundColor(.secondary)

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
                    .foregroundStyle(.secondary)
                    .fontWeight(.medium)
            }
        }
        .onTapGesture {
            self.sheetState = TaggedItem(.fee)
        }
        .padding(.top, 12)
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
                    Text(metadata.masterFingerprint?.asUppercase() ?? "No Fingerprint")
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
    var TotalSection: some View {
        HStack {
            Text("Total Spent")
                .font(.title3)
                .fontWeight(.medium)

            Spacer()

            Text(sendAmount)
                .multilineTextAlignment(.center)
                .font(.title3)
                .fontWeight(.medium)
        }
        .padding(.top, 12)
    }

    @ViewBuilder
    var NextButtonBottom: some View {
        Button(action: {
            // Action
        }) {
            Text("Next")
                .font(.title3)
                .fontWeight(.semibold)
                .frame(maxWidth: .infinity)
                .padding()
                .background(Color.midnightBlue)
                .foregroundColor(.white)
                .cornerRadius(10)
        }
        .padding(.top, 8)
        .padding(.bottom)
    }

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .qr:
            QrCodeAddressView(app: _app, scannedCode: $scannedCode)
                .presentationDetents([.large])
        case .fee:
            SendFlowSelectFeeRateView(
                feeOptions: feeRateOptions!,
                selectedOption: Binding(
                    get: { selectedFeeRate! },
                    set: { newValue in
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
            case .emptyAddress, .invalidAddress, .wrongNetwork: "Invalid Address"
            case .invalidNumber, .zeroAmount: "Invalid Amount"
            case .insufficientFunds: "Insufficient Funds"
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
                return "Can't send an empty transaction. Please enter a valid amount"
            case let .invalidAddress(address):
                return "The address \(address) is invalid"
            case let .wrongNetwork(address):
                return
                    "The address \(address) is on the wrong network. You are on \(metadata.network)"
            case .insufficientFunds:
                return "You do not have enough bitcoin in your wallet to cover the amount plus fees"
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
        case .invalidNumber, .insufficientFunds, .zeroAmount, .sendAmountToLow:
            Button("OK") { focusField = .amount }
        }
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
