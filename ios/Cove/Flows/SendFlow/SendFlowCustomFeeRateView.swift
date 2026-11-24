//
//  SendFlowCustomFeeRateView.swift
//  Cove
//
//  Created by Praveen Perera on 1/20/25.
//

import SwiftUI

func isTooHighCustomFeeError(_ error: Error) -> Bool {
    switch error {
    case SendFlowError.InsufficientFunds:
        true
    case SendFlowError.WalletManager(.InsufficientFunds(_)):
        true
    default:
        false
    }
}

/// The send flow's custom fee rate sheet, pricing each rate as the payment being built
struct SendFlowCustomFeeRateView: View {
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var manager
    @Environment(SendFlowManager.self) private var sendFlowManager
    @Environment(SendFlowPresenter.self) private var presenter

    // args
    @Binding var feeOptions: FeeRateOptionsWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee
    @Binding var selectedPresentationDetent: PresentationDetent

    private var policy: CustomFeeRatePolicy {
        CustomFeeRatePolicy(
            minimumFeeRate: 1,
            maximumFeeRate: feeOptions.fast().satPerVb() * 3,
            isBelowMinimum: { FeeRate.fromSatPerVb(satPerVb: $0).isBelowMinimum() },
            belowMinimumMessage: "The fee rate must be at least 1 sat/vB",
            price: price
        )
    }

    var body: some View {
        CustomFeeRateForm(
            title: "Set Custom Network Fee",
            unitLabel: "satoshi/byte",
            feeOptions: feeOptions,
            initialFeeRate: selectedOption.feeRate().satPerVb(),
            policy: policy,
            erroredFeeRate: presenter.erroredFeeRate,
            lastWorkingFeeRate: presenter.lastWorkingFeeRate,
            totalText: { "\($0.asSats()) sats" },
            totalDetail: feeInFiat,
            onPriced: priced,
            onPriceFailed: priceFailed,
            onDone: addCustomFeeOption
        )
        .padding(.horizontal)
        .padding(.vertical)
        .padding(.top, 22)
        .navigationBarBackButtonHidden()
    }

    private func feeInFiat(_ amount: Amount) -> String {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return ""
        }

        return "≈ \(manager.convertAndDisplayFiat(amount: amount, prices: prices))"
    }

    /// No payment to price until the address and amount are in
    private func price(_ feeRate: FeeRate, _ feeSpeed: FeeSpeed) async throws -> FeeRateOptionWithTotalFee? {
        if sendFlowManager.amount == nil { return nil }
        if sendFlowManager.address == nil { return nil }

        return try await sendFlowManager.getNewCustomFeeRateWithTotal(feeRate: feeRate, feeSpeed: feeSpeed)
    }

    private func priced(_ feeRateOption: FeeRateOptionWithTotalFee) {
        feeOptions = feeOptions.addCustomFeeRate(feeRate: feeRateOption)
        presenter.lastWorkingFeeRate = feeRateOption.feeRate().satPerVb()
    }

    private func priceFailed(_ error: Error, _ feeRate: FeeRate) {
        // the Rust Display text, since interpolating SendFlowError reflects its payload
        let errorText = (error as? SendFlowError)?.description ?? "\(error)"

        guard isTooHighCustomFeeError(error) else {
            Log.error("Unable to get accurate total sats \(errorText)")
            return
        }

        Log.error("Unable to get accurate total sats \(errorText), setting max fee rate to \(feeRate.satPerVb())")
        presenter.erroredFeeRate = feeRate.satPerVb()

        guard presenter.lastWorkingFeeRate != nil else { return }
        presenter.sheetState = .none
        Task {
            try? await Task.sleep(for: .milliseconds(850))
            await MainActor.run {
                presenter.alertState = .init(.general(title: "Fee too high!", message: "The fee rate you entered is too high, we automatically selected a lower fee"))
            }
        }
    }

    private func addCustomFeeOption(_ feeRate: Float) {
        // if there is an non custom fee option for the same fee rate select it and remove the custom fee option
        if let newSelectedOption = feeOptions.getFeeRateWith(feeRate: feeRate),
           !newSelectedOption.isCustom()
        {
            Log.debug(
                "removing custom fee option (fee rate: \(feeRate)), selected: \(newSelectedOption.feeSpeed().string)"
            )
            presenter.sheetState = .none
            feeOptions = feeOptions.removeCustomFee()
            selectedOption = newSelectedOption
            return
        }

        if let customOption = feeOptions.custom() {
            selectedPresentationDetent = .height(550)
            selectedOption = customOption
        }

        presenter.sheetState = .none
    }
}

#Preview {
    AsyncPreview {
        VStack {
            SendFlowCustomFeeRateView(
                feeOptions: Binding.constant(FeeRateOptionsWithTotalFee.previewNew()),
                selectedOption: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().medium()
                ),
                selectedPresentationDetent: Binding.constant(PresentationDetent.large)
            )
            .environment(WalletManager(preview: .only))
            .environment(AppManager.shared)
            .environment(
                SendFlowPresenter(
                    app: AppManager.shared, manager: WalletManager(preview: .only)
                )
            )
            .frame(height: 300)
            .background(.coveBg)
        }
        .frame(maxHeight: .infinity)
        .background(.black)
    }
}
