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

    var sliderBinding: Binding<Float> {
        Binding(
            get: {
                Float(feeRate) ?? selectedOption.satPerVb()
            },
            set: {
                feeRate = String(format: "%.2f", $0)
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
        guard let address = presenter.address else { return }
        guard let amount = presenter.amount else { return }

        let feeRate = FeeRate.fromSatPerVb(satPerVb: Float(feeRate))
        let isMaxSelected = presenter.maxSelected != nil

        if let totalSatsTask { totalSatsTask.cancel() }
        totalSatsTask = Task {
            try? await Task.sleep(for: .milliseconds(50))
            if Task.isCancelled { return }

            do {
                let psbt =
                    if isMaxSelected {
                        try await manager.rust.buildDrainTransaction(address: address, fee: feeRate)
                    } else {
                        try await manager.rust.buildTransactionWithFeeRate(
                            amount: amount, address: address, feeRate: feeRate
                        )
                    }

                let totalFee = try psbt.fee()
                let totalFeeSats = totalFee.asSats()
                totalSats = Int(totalFeeSats)
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

        let feeOptions = feeOptions.addCustomFee(feeRate: Float(feeRate))
        self.feeOptions = feeOptions

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
                    Slider(value: sliderBinding, in: 1 ... feeOptions.fast().satPerVb() * 2)
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
