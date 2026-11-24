//
//  BumpFeeCustomRateSheet.swift
//  Cove
//
//  Created by Praveen Perera on 11/23/26.
//

import SwiftUI

/// Sets a custom rate for a fee bump, which can't go below the lowest rate a replacement needs
struct BumpFeeCustomRateSheet: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss

    let manager: FeeBumpManager
    let walletManager: WalletManager
    let options: FeeBumpOptions

    /// The last rate Rust priced, which Done selects when it is the rate typed
    @State private var priced: FeeRateOptionWithTotalFee?

    /// Set once Done or the X closed the sheet, after which its going away selects nothing, and
    /// the X leaves the selection as it was
    @State private var isFinished = false

    private var fiat: FeeBumpFiat {
        FeeBumpFiat(app: app, manager: walletManager)
    }

    private var minimumFeeRate: Float {
        options.minimumFeeRate.satPerVb()
    }

    private var initialFeeRate: Float {
        max(manager.selected?.feeRate().satPerVb() ?? minimumFeeRate, minimumFeeRate)
    }

    private var policy: CustomFeeRatePolicy {
        let minimum = minimumFeeRate

        return CustomFeeRatePolicy(
            minimumFeeRate: minimum,
            maximumFeeRate: options.feeOptions.fast().satPerVb() * 3,
            isBelowMinimum: { $0 < minimum },
            belowMinimumMessage: "Must be at least \(options.minimumFeeRate.satsPerVbyteString()) to replace your current fee",
            showsMinimumHint: true,
            price: { feeRate, _ in try await manager.customFeeOption(feeRate) },
            priceFailureMessage: FeeBumpManager.customRateFailureMessage
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            FeeBumpSheetCloseButton(action: cancel)

            CustomFeeRateForm(
                title: nil,
                unitLabel: "sats/vbyte",
                feeOptions: options.feeOptions,
                initialFeeRate: initialFeeRate,
                policy: policy,
                totalText: { "\($0.satsString()) sats" },
                totalDetail: totalDetail,
                onPriced: { priced = $0 },
                onPriceFailed: { error, feeRate in
                    Log.warn("Unable to price a fee bump at \(feeRate.satPerVb()) sat/vB: \(error)")
                },
                onDone: done
            )
            .padding(.horizontal)
        }
        .background(Color.coveBg)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("speedUp.customSheet")
    }

    /// The fiat value and how much more than the current fee, as "≈ 1.34 USD · +400%"
    private func totalDetail(_ total: Amount) -> String {
        let original = options.originalFee.asSats()
        let increase = original == 0 ? nil : (Double(total.asSats()) - Double(original)) / Double(original) * 100
        let percent = increase.map { "+\(Int($0.rounded()))%" }

        return [fiat.approximate(total), percent].compactMap(\.self).joined(separator: " · ")
    }

    private func cancel() {
        isFinished = true
        dismiss()
    }

    private func done(_ feeRate: Float) {
        guard !isFinished else { return }
        isFinished = true
        defer { dismiss() }

        if let priced, priced.feeRate().satPerVb() == feeRate {
            manager.select(priced)
            return
        }

        // Done came before the typed rate finished pricing
        manager.selectCustomFeeRate(FeeRate.fromSatPerVb(satPerVb: feeRate))
    }
}
