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
    @Environment(SendFlowSetAmountPresenter.self) private var presenter

    // args
    @Binding var feeOptions: FeeRateOptionsWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee
    @Binding var selectedPresentationDetent: PresentationDetent

    // private
    @State private var feeRate: String = "4.46"
    @State private var txnSize: Int? = nil
    @State private var loaded = false

    // total sats
    @State private var accurateTotalSats: Int? = nil
    @State private var accurateTotalSatsTask: Task<Void, Never>?

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

    var totalSats: Int {
        let txnSize = txnSize ?? Int(feeOptions.transactionSize())
        guard let feeRate = Double(feeRate) else { return 0 }

        return Int(Double(txnSize) * feeRate)
    }

    var totalSatsString: String {
        if let accurateTotalSats {
            return "\(accurateTotalSats) sats"
        }

        return "\(totalSats) sats"
    }

    var feeInFiat: String {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return ""
        }

        return "≈ \(manager.rust.convertAndDisplayFiat(amount: Amount.fromSat(sats: UInt64(totalSats)), prices: prices))"
    }

    var feeSpeed: FeeSpeed {
        let feeRate = Double(feeRate) ?? 20.0
        return feeOptions.calculateCustomFeeSpeed(feeRate: Float(feeRate))
    }

    func getTotalSatsDeduped(for feeRate: Double) {
        guard let address = presenter.address else { return }
        guard let amount = presenter.amount else { return }
        let feeRate = FeeRate.fromSatPerVb(satPerVb: Float(feeRate))

        if let accurateTotalSatsTask {
            accurateTotalSatsTask.cancel()
        }

        accurateTotalSatsTask = Task {
            try? await Task.sleep(for: .milliseconds(50))

            do {
                let psbt = try await manager.rust.buildTransactionWithFeeRate(amount: amount, address: address, feeRate: feeRate)
                let totalFee = try psbt.fee()
                let totalFeeSats = totalFee.asSats()
                txnSize = Int(psbt.weight())
                accurateTotalSats = Int(totalFeeSats)
            }
            catch {
                Log.error("Unable to get accurate total sats \(error)")
            }
        }
    }

    func addCustomFeeOption() {
        guard let feeRate = Double(feeRate) else { return }

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
        guard let feeRate = Double(newFeeRate) else { return }
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
                    Text(totalSatsString)
                        .font(.caption)
                        .fontWeight(.semibold)

                    Text(feeInFiat)
                        .font(.caption2)
                        .foregroundStyle(.secondary)

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
            txnSize = Int(feeOptions.transactionSize())
            withAnimation { loaded = true }
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
            .environment(AppManager())
            .environment(SendFlowSetAmountPresenter(app: AppManager(), manager: WalletManager(preview: "preview_only")))
            .frame(height: 300)
            .background(.coveBg)
        }
        .frame(maxHeight: .infinity)
        .background(.black)
    }
}
