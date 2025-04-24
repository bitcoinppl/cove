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
    @Environment(SendFlowManager.self) private var sendFlowManager
    @Environment(WalletManager.self) private var manager

    @Environment(\.colorScheme) private var colorScheme

    let id: WalletId

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

    private var metadata: WalletMetadata {
        manager.walletMetadata
    }

    private var network: Network {
        metadata.network
    }

    private var totalSpentInFiat: String {
        // TODO: add total spent in fiat depending on metadata.selectedUnit
//        guard let totalSpentInBtc else { return "---" }
//        guard let prices = app.prices else { return "---" }
//
//        let fiat = totalSpentInBtc * Double(prices.get())
//        return "≈ \(manager.rust.displayFiatAmount(amount: fiat))"
        ""
    }

    private var totalSending: String {
        // TODO: add total sending in btc depending on metadata.selectedUnit
        ""
    }

    // MARK: Actions

    // validate, create final psbt and send to next screen
    private func next() {
        // TODO:
    }

    // doing it this way prevents an alert popping up when the user just goes back
    private func setAlertState(_ alertState: AlertState) {
        presenter.setAlertState(alertState)
    }

    var selectedFeeRate: FeeRateOptionWithTotalFee? {
        sendFlowManager.selectedFeeRate
    }

    var feeRateOptions: FeeRateOptionsWithTotalFee? {
        sendFlowManager.feeRateOptions
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
                        EnterAmountView()

                        // Address Section
                        VStack {
                            Divider()
                            EnterAddressView(address: $address)
                            Divider()
                        }

                        // Account Section
                        AccountSection

                        // TODO:
//                        if feeRateOptions != nil,
//                           selectedFeeRate != nil,
//                           Address.isValid(address)
//                        {
//                            // Network Fee Section
//                            NetworkFeeSection
//
//                            // Total Spending Section
//                            TotalSpendingSection
//
//                            // Next Button
//                            NextButtonBottom
//                        }
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
        .task {
            // TODO:
//            guard let feeRateOptions = try? await manager.rust.getFeeOptions() else {
//                return
//            }
//
//            await MainActor.run {
//                feeRateOptionsBase = feeRateOptions
//            }
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

            // TODO:
//            await MainActor.run {
//                if sendAmount == "0" || sendAmount == "" {
//                    presenter.focusField = .amount
//                    return
//                }
//
//                if address == "" {
//                    presenter.focusField = .address
//                    return
//                }
//            }
        }
        .onAppear {
            if metadata.walletType == .watchOnly {
                app.alertState = .init(.cantSendOnWatchOnlyWallet)
                app.popRoute()
                return
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

    private var totalFeeString: String {
        // TODO:
        ""
    }

    private var totalSpent: String {
        // TODO:
        ""
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
        _: String? = nil, displayAlert _: Bool = false
    ) -> Bool {
        // TODO:
//        let sendAmountRaw = amount ?? sendAmount
//        if displayAlert {
//            Log.debug("validating amount: \(sendAmount)")
//        }
//
//        let sendAmount = sendAmountRaw.replacingOccurrences(of: ",", with: "")
//        guard let amount = Double(sendAmount) else {
//            if displayAlert { setAlertState(.invalidNumber) }
//            return false
//        }
//
//        let balance = Double(manager.balance.spendable().asSats())
//        let amountSats = amountSats(amount)
//
//        if amountSats < 5000 {
//            if displayAlert { setAlertState(.sendAmountToLow) }
//            return false
//        }
//
//        if amountSats > balance {
//            if displayAlert { setAlertState(.insufficientFunds) }
//            return false
//        }
//
//        if let selectedFeeRate {
//            let totalFeeSats = Double(selectedFeeRate.totalFee().asSats())
//            if (amountSats + totalFeeSats) > balance {
//                if displayAlert { setAlertState(.insufficientFunds) }
//                return false
//            }
//        }

        true
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
        // TODO: clear send amount

//        if metadata.fiatOrBtc == .fiat {
//            Log.debug("fiat \(sendAmountFiat)")
//            sendAmountFiat = app.selectedFiatCurrency.symbol()
//            sendAmount = "0"
//            return
//        }
//
//        if metadata.fiatOrBtc == .btc {
//            sendAmount = ""
//            sendAmountFiat = manager.rust.displayFiatAmount(amount: 0.0)
//            return
//        }
    }

    // MARK: OnChange Functions

    private func selectedUnitChanged(_: Unit, _: Unit) {
//    TODO: selectedUnitChanged
//        let sendAmount = sendAmount.replacingOccurrences(of: ",", with: "")
//        guard let amount = Double(sendAmount) else { return }
//        if amount == 0 { return }
//        if oldUnit == newUnit { return }
//
//        switch newUnit {
//        case .btc:
//            self.sendAmount = Double(amount / 100_000_000).btcFmt()
//        case .sat:
//            let sendAmount = Int(amount * 100_000_000)
//            if presenter.focusField == .address || presenter.focusField == .none {
//                self.sendAmount = ThousandsFormatter(sendAmount).fmt()
//            } else {
//                self.sendAmount = String(sendAmount)
//            }
//        }
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

    private func setMaxSelected(_: FeeRateOptionWithTotalFee? = nil) {
        // TODO:
    }

    private func scannedCodeChanged(_: TaggedString?, _: TaggedString?) {
        // TODO:
//        guard let newValue else { return }
//        presenter.sheetState = nil
//
//        let addressWithNetwork = try? AddressWithNetwork(address: newValue.item)
//
//        guard let addressWithNetwork else {
//            setAlertState(.invalidAddress(newValue.item))
//            return
//        }
//
//        address = addressWithNetwork.address().string()
//        guard validateAddress(address, displayAlert: true) else { return }
//
//        // TODO:
//        if let amount = addressWithNetwork.amount() {
        ////            setAmount(amount)
//            if !validateAmount(displayAlert: true) {
//                presenter.focusField = .amount
//                return
//            }
//        }
//
//        if sendAmount == "0" || sendAmount == "" || !validateAmount() {
//            return DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
//                presenter.focusField = .amount
//            }
//        }
//
//        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
//            presenter.focusField = .none
//        }
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

            Button(action: { setMaxSelected() }) {
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
            // TODO:
            EmptyView()
//            SendFlowSelectFeeRateView(
//                manager: manager,
//                feeOptions: Binding(get: { feeRateOptions! }, set: { /* feeRateOptions = $0 */ }),
//                selectedOption: Binding(
//                    get: { selectedFeeRate! },
//                    set: { newValue in
//                        // in maxSelected mode, so adjust with new rate
//                        if presenter.maxSelected != nil {
//                            setMaxSelected(newValue)
//                        }
//
            ////                        selectedFeeRate = newValue
//                    }
//                ),
//                selectedPresentationDetent: $selectedPresentationDetent
//            )
//            .presentationDetents(
//                [.height(440), .height(550), .large],
//                selection: $selectedPresentationDetent
//            )
        }
    }
}

#Preview("with address") {
    AsyncPreview {
        NavigationStack {
            let manager = WalletManager(preview: "preview_only")

            SendFlowSetAmountScreen(
                id: WalletId(),
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
                address: ""
            )
            .environment(manager)
            .environment(AppManager.shared)
            .environment(SendFlowPresenter(app: AppManager.shared, manager: manager))
        }
    }
}
