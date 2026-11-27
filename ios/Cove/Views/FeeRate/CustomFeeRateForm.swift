//
//  CustomFeeRateForm.swift
//  Cove
//
//  Created by Praveen Perera on 11/23/26.
//

import SwiftUI

/// What a custom fee rate may be, and how to price one, for [`CustomFeeRateForm`]
///
/// Plain data and closures, so the send flow and the bump fee sheet each keep their own rules
/// for a rate while sharing the form
struct CustomFeeRatePolicy {
    /// The lowest rate the slider reaches
    let minimumFeeRate: Float

    /// The highest rate the slider reaches, before any rate that failed to price lowers it
    let maximumFeeRate: Float

    /// Whether a typed rate is too low to use, which disables Done
    let isBelowMinimum: (Float) -> Bool

    /// Shown in red under the field while the rate is too low
    let belowMinimumMessage: String

    /// Shows the minimum message in gray while the rate is fine too, as a standing hint
    var showsMinimumHint: Bool = false

    /// The total fee at a rate, `nil` when it can't be priced yet
    let price: (FeeRate, FeeSpeed) async throws -> FeeRateOptionWithTotalFee?

    /// What to show in red for a rate that failed to price, which also disables Done for it;
    /// `nil` leaves the failure to `onPriceFailed` alone
    var priceFailureMessage: (Error) -> String? = { _ in nil }
}

/// Edits a custom fee rate: a sats/vB field, a slider, how long the rate should take to
/// confirm and the total fee it pays
struct CustomFeeRateForm: View {
    let title: String?
    let unitLabel: String
    let feeOptions: FeeRateOptionsWithTotalFee
    let initialFeeRate: Float
    let policy: CustomFeeRatePolicy

    /// A rate above this one failed to price, so the slider stops just above it
    var erroredFeeRate: Float? = nil

    /// The last rate that priced, which the slider falls back to past the errored rate
    var lastWorkingFeeRate: Float? = nil

    /// The total fee as the form shows it, such as "1,410 sats"
    let totalText: (Amount) -> String

    /// What follows the total, such as "≈ 1.34 USD · +400%"
    let totalDetail: (Amount) -> String

    let onPriced: (FeeRateOptionWithTotalFee) -> Void
    var onPriceFailed: (Error, FeeRate) -> Void = { _, _ in }

    /// Called with a usable rate when the user taps Done, and again when the form goes away
    let onDone: (Float) -> Void

    @State private var feeRate: String = ""
    @State private var loaded = false
    @State private var total: Amount? = nil
    @State private var priceFailure: PriceFailure? = nil
    @State private var priceTask: Task<Void, Never>?

    private struct PriceFailure {
        let feeRate: Float
        let message: String
    }

    private var typedFeeRate: Float? {
        Float(feeRate)
    }

    private var isBelowMinimum: Bool {
        guard let typedFeeRate else { return false }
        return policy.isBelowMinimum(typedFeeRate)
    }

    /// The failure of the rate typed, when it failed to price
    private var typedPriceFailure: PriceFailure? {
        guard let priceFailure, priceFailure.feeRate == typedFeeRate else { return nil }
        return priceFailure
    }

    private var feeSpeed: FeeSpeed {
        feeOptions.calculateCustomFeeSpeed(feeRate: typedFeeRate ?? 20.0)
    }

    private var sliderUpperBound: Float {
        let upper = min(erroredFeeRate.map { $0 + 0.01 } ?? policy.maximumFeeRate, policy.maximumFeeRate)

        // a range must not be empty, which a minimum at or above the maximum would make it
        return max(upper, policy.minimumFeeRate + 0.01)
    }

    private var sliderBinding: Binding<Float> {
        Binding(
            get: {
                let feeRate = typedFeeRate ?? initialFeeRate
                return (feeRate * 100).rounded() / 100
            },
            set: {
                var feeRate = $0
                if let erroredFeeRate, feeRate > erroredFeeRate {
                    let minimum = policy.minimumFeeRate
                    feeRate = max(erroredFeeRate - 0.02, lastWorkingFeeRate.map { $0 - 0.01 } ?? minimum, minimum)
                }

                self.feeRate = String(format: "%.2f", feeRate)
            }
        )
    }

    var body: some View {
        VStack(spacing: 20) {
            if let title {
                Text(title)
                    .font(.title3)
                    .fontWeight(.bold)
                    .padding(.vertical, 12)
            }

            CustomFeeRateEditor(
                unitLabel: unitLabel,
                feeRate: $feeRate,
                feeSpeed: feeSpeed,
                sliderValue: sliderBinding,
                sliderRange: policy.minimumFeeRate ... sliderUpperBound,
                total: total,
                showsTotal: typedPriceFailure == nil,
                totalText: totalText,
                totalDetail: totalDetail,
                minimumMessage: minimumMessage,
                minimumMessageBelowTotal: policy.showsMinimumHint
            )

            Divider()

            CustomFeeRateDoneButton(action: done, isDisabled: isBelowMinimum || typedPriceFailure != nil)
        }
        .onChange(of: feeRate, initial: true, feeRateChanged)
        .onAppear(perform: loadInitialFeeRate)
        .onDisappear(perform: done)
        .opacity(loaded ? 1 : 0)
    }

    private var minimumMessage: CustomFeeRateEditor.MinimumMessage? {
        if isBelowMinimum { return .error(policy.belowMinimumMessage) }
        if let typedPriceFailure { return .error(typedPriceFailure.message) }
        if policy.showsMinimumHint { return .hint(policy.belowMinimumMessage) }
        return nil
    }

    private func loadInitialFeeRate() {
        feeRate = String(initialFeeRate)
        withAnimation { loaded = true }
    }

    private func done() {
        guard let typedFeeRate, !isBelowMinimum, typedPriceFailure == nil else { return }
        onDone(typedFeeRate)
    }

    private func feeRateChanged(_: String?, newFeeRate: String?) {
        guard let newFeeRate, let feeRate = Float(newFeeRate), !isBelowMinimum else {
            // a pending request for the previous rate must not update totals or dismiss the sheet
            priceTask?.cancel()
            priceTask = nil
            return
        }

        priceDeduped(feeRate)
    }

    private func priceDeduped(_ feeRate: Float) {
        let rate = FeeRate.fromSatPerVb(satPerVb: feeRate)
        let speed = feeSpeed

        priceTask?.cancel()
        priceTask = Task {
            try? await Task.sleep(for: .milliseconds(50))
            if Task.isCancelled { return }

            do {
                guard let option = try await policy.price(rate, speed) else { return }

                // the rate may have changed or become invalid while Rust priced this one
                if Task.isCancelled { return }

                await MainActor.run {
                    if let totalFee = option.totalFee() { total = totalFee }
                    priceFailure = nil
                    onPriced(option)
                }
            } catch {
                if Task.isCancelled { return }

                await MainActor.run {
                    // the last total belongs to another rate, so it must not stand for this one
                    total = nil
                    priceFailure = policy.priceFailureMessage(error).map { PriceFailure(feeRate: feeRate, message: $0) }
                    onPriceFailed(error, rate)
                }
            }
        }
    }
}

private struct CustomFeeRateEditor: View {
    enum MinimumMessage {
        case hint(String)
        case error(String)
    }

    let unitLabel: String
    @Binding var feeRate: String

    let feeSpeed: FeeSpeed
    let sliderValue: Binding<Float>
    let sliderRange: ClosedRange<Float>
    let total: Amount?

    /// Off while the rate failed to price, as its message says why there is no total
    let showsTotal: Bool
    let totalText: (Amount) -> String
    let totalDetail: (Amount) -> String
    let minimumMessage: MinimumMessage?

    /// A standing hint reads as a note on the total, an error alone sits right under the field
    let minimumMessageBelowTotal: Bool

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Text(unitLabel)
                    .fontWeight(.medium)
                    .foregroundStyle(.secondary)
                    .font(.callout)

                Spacer()
            }
            .offset(y: 4)

            HStack {
                TextField(feeRate, text: $feeRate)
                    .keyboardType(.decimalPad)
                    .font(.system(size: 34, weight: .semibold))
                    .accessibilityIdentifier("customFeeRate.field")

                Spacer()

                SendFlowDurationCapsule(
                    speed: feeSpeed,
                    fontColor: .primary,
                    font: .footnote,
                    fontWeight: .semibold
                )
            }

            if let minimumMessage, !minimumMessageBelowTotal {
                CustomFeeRateMinimumMessage(message: minimumMessage)
            }

            Slider(value: sliderValue, in: sliderRange, step: 0.01)
                .accessibilityIdentifier("customFeeRate.slider")

            if showsTotal {
                CustomFeeRateTotal(total: total, totalText: totalText, totalDetail: totalDetail)
            }

            if let minimumMessage, minimumMessageBelowTotal {
                CustomFeeRateMinimumMessage(message: minimumMessage)
            }
        }
    }
}

private struct CustomFeeRateMinimumMessage: View {
    let message: CustomFeeRateEditor.MinimumMessage

    var body: some View {
        HStack {
            switch message {
            case let .hint(text):
                Text(text)
                    .foregroundStyle(.secondary)
            case let .error(text):
                Text(text)
                    .foregroundStyle(.red)
            }

            Spacer()
        }
        .font(.caption)
        .accessibilityIdentifier("customFeeRate.minimumMessage")
    }
}

private struct CustomFeeRateTotal: View {
    let total: Amount?
    let totalText: (Amount) -> String
    let totalDetail: (Amount) -> String

    var body: some View {
        HStack {
            AsyncText(text: total.map(totalText), font: .caption, spinnerScale: 0.7)
                .fontWeight(.semibold)
                .accessibilityIdentifier("customFeeRate.total")

            if let total {
                Text(totalDetail(total))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            Spacer()
        }
    }
}

private struct CustomFeeRateDoneButton: View {
    let action: () -> Void
    let isDisabled: Bool

    var body: some View {
        Button(action: action) {
            Text("Done")
                .font(.footnote)
                .fontWeight(.semibold)
                .frame(maxWidth: .infinity)
                .padding()
        }
        .background(Color.midnightBtn)
        .foregroundColor(.white)
        .cornerRadius(10)
        .disabled(isDisabled)
        .opacity(isDisabled ? 0.5 : 1)
        .padding(.horizontal, detailsExpandedPadding)
        .padding(.top, 14)
        .accessibilityIdentifier("customFeeRate.done")
    }
}
