//
//  SendFlowCustomFeeRateView.swift
//  Cove
//
//  Created by Praveen Perera on 1/20/25.
//

import SwiftUI

struct SendFlowCustomFeeRateView: View {
    @Environment(AppManager.self) private var app
    @Environment(WalletManager.self) private var manager
    @Environment(SendFlowManager.self) private var sendFlowManager
    @Environment(SendFlowPresenter.self) private var presenter

    // args
    @Binding var feeOptions: FeeRateOptionsWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee
    @Binding var selectedPresentationDetent: PresentationDetent

    // private
    @State private var feeRate: String = "4.46"
    @State private var loaded = false

    // total sats
    @State private var totalSats: Int? = nil
    @State private var totalSatsTask: Task<Void, Never>?

    var erroredFeeRate: Float? {
        presenter.erroredFeeRate
    }

    var lastWorkingFeeRate: Float? {
        presenter.lastWorkingFeeRate
    }

    var sliderBinding: Binding<Float> {
        Binding(
            get: {
                let feeRate = Float(feeRate) ?? selectedOption.satPerVb()
                let clampedFeeRate = (feeRate * 100).rounded() / 100
                return clampedFeeRate
            },
            set: {
                var feeRate = $0
                if let erroredFeeRate, feeRate > erroredFeeRate {
                    feeRate = max(erroredFeeRate - 0.02, lastWorkingFeeRate.map { $0 - 0.01 } ?? 1, 1)
                }

                self.feeRate = String(format: "%.2f", feeRate)
            }
        )
    }

    var totalSatsString: String {
        if let totalSats {
            return "\(totalSats) sats"
        }

        return ""
    }

    var feeInFiat: String {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return ""
        }

        return
            "≈ \(manager.rust.convertAndDisplayFiat(amount: Amount.fromSat(sats: UInt64(totalSats ?? 0)), prices: prices))"
    }

    var feeSpeed: FeeSpeed {
        let feeRate = Float(feeRate) ?? 20.0
        return feeOptions.calculateCustomFeeSpeed(feeRate: Float(feeRate))
    }

    func getTotalSatsDeduped(for feeRate: Float) {
        if sendFlowManager.amount == nil { return }
        if sendFlowManager.address == nil { return }

        let feeRate = FeeRate.fromSatPerVb(satPerVb: Float(feeRate))

        if let totalSatsTask { totalSatsTask.cancel() }
        totalSatsTask = Task {
            try? await Task.sleep(for: .milliseconds(50))
            if Task.isCancelled { return }

            do {
                let feeRateOption = try await sendFlowManager.getNewCustomFeeRateWithTotal(
                    feeRate: feeRate, feeSpeed: feeSpeed
                )

                if let totalFee = feeRateOption.totalFee() {
                    self.totalSats = Int(totalFee.asSats())
                }
                await MainActor.run {
                    let feeOptions = feeOptions.addCustomFeeRate(feeRate: feeRateOption)
                    self.feeOptions = feeOptions
                    presenter.lastWorkingFeeRate = feeRate.satPerVb()
                }
            } catch let SendFlowError.WalletManager(.InsufficientFunds(error)) {
                Log.error("Unable to get accurate total sats \(error), setting max fee rate to \(feeRate.satPerVb())")
                let feeRate = feeRate.satPerVb()

                await MainActor.run { presenter.erroredFeeRate = feeRate }

                guard lastWorkingFeeRate != nil else { return }
                await MainActor.run { presenter.sheetState = .none }
                Task {
                    try? await Task.sleep(for: .milliseconds(850))
                    await MainActor.run {
                        presenter.alertState = .init(.general(title: "Fee too high!", message: "The fee rate you entered is too high, we automatically selected a lower fee"))
                    }
                }
            } catch {
                Log.error("Unable to get accurate total sats \(error)")
            }
        }
    }

    func addCustomFeeOption() {
        guard let feeRate = Float(feeRate) else { return }

        // if there is an non custom fee option for the same fee rate select it and remove the custom fee option
        if let newSelectedOption = feeOptions.getFeeRateWith(feeRate: feeRate),
           !newSelectedOption.isCustom()
        {
            Log.debug(
                "removing custom fee option (fee rate: \(feeRate)), selected: \(newSelectedOption.feeSpeed().string)"
            )
            presenter.sheetState = .none
            self.feeOptions = feeOptions.removeCustomFee()
            selectedOption = newSelectedOption
            return
        }

        if let customOption = feeOptions.custom() {
            selectedPresentationDetent = .height(550)
            selectedOption = customOption
        }

        presenter.sheetState = .none
    }

    func feeRateChanged(_: String?, newFeeRate: String?) {
        guard let newFeeRate else { return }
        guard let feeRate = Float(newFeeRate) else { return }
        getTotalSatsDeduped(for: feeRate)
    }

    var body: some View {
        VStack(spacing: 20) {
            Text("Set Custom Network Fee")
                .font(.title3)
                .fontWeight(.bold)
                .padding(.vertical, 12)

            VStack(spacing: 8) {
                HStack {
                    Text("satoshi/byte")
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

                    Spacer()

                    SendFlowDurationCapsule(
                        speed: feeSpeed,
                        fontColor: .primary,
                        font: .footnote,
                        fontWeight: .semibold
                    )
                }

                HStack {
                    let fast3 = feeOptions.fast().satPerVb() * 3
                    if let erroredFeeRate {
                        Slider(value: sliderBinding, in: 1 ... min(erroredFeeRate + 0.01, fast3), step: 0.01)
                    } else {
                        Slider(value: sliderBinding, in: 1 ... fast3, step: 0.01)
                    }
                }

                HStack {
                    if totalSats == nil {
                        ProgressView()
                            .controlSize(.mini)
                            .tint(.primary)
                    } else {
                        Text(totalSatsString)
                            .font(.caption)
                            .fontWeight(.semibold)

                        Text(feeInFiat)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }

                    Spacer()
                }
            }

            Divider()

            Button(action: addCustomFeeOption) {
                Text("Done")
                    .font(.footnote)
                    .fontWeight(.semibold)
                    .frame(maxWidth: .infinity)
                    .padding()
            }
            .background(Color.midnightBtn)
            .foregroundColor(.white)
            .cornerRadius(10)
            .padding(.horizontal, detailsExpandedPadding)
            .padding(.top, 14)
        }
        .padding(.horizontal)
        .padding(.vertical)
        .padding(.top, 22)
        .onChange(of: feeRate, initial: true, feeRateChanged)
        .onAppear {
            feeRate = String(selectedOption.feeRate().satPerVb())
            withAnimation { loaded = true }
        }
        .onDisappear {
            addCustomFeeOption()
        }
        .navigationBarBackButtonHidden()
        .opacity(loaded ? 1 : 0)
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
            .environment(WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
            .environment(
                SendFlowPresenter(
                    app: AppManager.shared, manager: WalletManager(preview: "preview_only")
                )
            )
            .frame(height: 300)
            .background(.coveBg)
        }
        .frame(maxHeight: .infinity)
        .background(.black)
    }
}
