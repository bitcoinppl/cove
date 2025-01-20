//
//  SendFlowSelectFeeRateView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import Foundation
import SwiftUI

struct SendFlowSelectFeeRateView: View {
    enum Screen { case select, custom }

    let manager: WalletManager

    @Binding var feeOptions: FeeRateOptionsWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee
    @Binding var selectedPresentationDetent: PresentationDetent

    // private
    @State private var route: [Screen] = []

    @ViewBuilder
    var SelectView: some View {
        VStack(spacing: 20) {
            Text("Network Fee")
                .font(.title3)
                .fontWeight(.bold)
                .padding(.vertical, 8)

            FeeOptionView(
                manager: manager,
                feeOption: feeOptions.fast(),
                selectedOption: $selectedOption
            )

            FeeOptionView(
                manager: manager,
                feeOption: feeOptions.medium(),
                selectedOption: $selectedOption
            )

            FeeOptionView(
                manager: manager,
                feeOption: feeOptions.slow(),
                selectedOption: $selectedOption
            )

            if let custom = feeOptions.custom() {
                FeeOptionView(
                    manager: manager,
                    feeOption: custom,
                    selectedOption: $selectedOption
                )
            }

            Button(action: {
                route = [.custom]
            }) {
                Text("Customize Fee")
                    .font(.subheadline)
                    .fontWeight(.semibold)
            }
            .frame(maxWidth: .infinity)
            .padding()
            .background(Color.midnightBtn)
            .foregroundColor(.white)
            .cornerRadius(10)
            .padding(.horizontal, detailsExpandedPadding)
            .padding(.vertical, 12)
        }
        .padding(.horizontal)
        .padding(.top, 22)
    }

    var body: some View {
        NavigationStack(path: $route) {
            SelectView
                .navigationDestination(
                    for: Screen.self,
                    destination: { route in
                        switch route {
                        case .custom: CustomRateFee(
                                feeOptions: $feeOptions,
                                selectedOption: $selectedOption,
                                selectedPresentationDetent: $selectedPresentationDetent
                            )
                        case .select: SelectView
                        }
                    }
                )
        }
    }
}

private struct FeeOptionView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss

    // passed in args
    let manager: WalletManager
    let feeOption: FeeRateOptionWithTotalFee
    @Binding var selectedOption: FeeRateOptionWithTotalFee

    var isSelected: Bool {
        selectedOption.feeSpeed() == feeOption.feeSpeed()
    }

    var fontColor: Color {
        if isSelected { .white } else { .primary }
    }

    var strokeColor: Color {
        if isSelected { Color.midnightBtn } else { Color.secondary }
    }

    var totalFee: String {
        feeOption.totalFee().satsString()
    }

    var satsPerVbyte: Double {
        Double(feeOption.satPerVb())
    }

    private var fiatAmount: String {
        guard let prices = app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = feeOption.totalFee()
        return "≈ \(manager.rust.convertAndDisplayFiat(amount: amount, prices: prices))"
    }

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(String(feeOption.feeSpeed()))
                        .font(.headline)
                        .foregroundColor(fontColor)

                    DurationCapsule(
                        speed: feeOption.feeSpeed(), fontColor: fontColor
                    )
                    .font(.caption2)
                }

                HStack {
                    Text("\(String(format: "%.2f", satsPerVbyte)) sats/vbyte")
                        .font(.subheadline)
                        .foregroundColor(fontColor)
                }
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 4) {
                Text("\(totalFee) sats")
                    .font(.headline)
                    .foregroundColor(fontColor)

                Text(fiatAmount)
                    .font(.subheadline)
                    .foregroundColor(fontColor)
            }
        }
        .padding()
        .background(
            isSelected
                ? Color.midnightBtn.opacity(0.8) : Color(UIColor.systemGray6)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .stroke(strokeColor, lineWidth: 1)
        )
        .onTapGesture {
            selectedOption = feeOption
            dismiss()
        }
        .cornerRadius(12)
    }
}

private struct DurationCapsule: View {
    let speed: FeeSpeed
    let fontColor: Color
    var font: Font = .subheadline
    var fontWeight: Font.Weight = .regular

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(speed.circleColor)
                .frame(width: 8, height: 8)
            Text(speed.duration)
        }
        .font(font)
        .fontWeight(fontWeight)
        .foregroundColor(fontColor)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Color.gray.opacity(0.2))
        .cornerRadius(8)
    }
}

private struct CustomRateFee: View {
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
        "\(totalSats) sats"
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

                    DurationCapsule(
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
        .onAppear {
            feeRate = String(selectedOption.feeRate().satPerVb())
            txnSize = Int(feeOptions.transactionSize())
            withAnimation { loaded = true }
        }
        .navigationBarBackButtonHidden()
        .opacity(loaded ? 1 : 0)
    }
}

#Preview("Select Fee Rate") {
    AsyncPreview {
        VStack {
            SendFlowSelectFeeRateView(
                manager: WalletManager(preview: "preview_only"),
                feeOptions: Binding.constant(FeeRateOptionsWithTotalFee.previewNew()),
                selectedOption: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().medium()
                ),
                selectedPresentationDetent: Binding.constant(PresentationDetent.large)
            )
            .environment(WalletManager(preview: "preview_only"))
            .environment(AppManager())
            .frame(height: 440)
        }
        .frame(maxHeight: .infinity)
        .background(.coveBg)
    }
}

#Preview("Custom Fee Rate") {
    AsyncPreview {
        VStack {
            CustomRateFee(
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

#Preview("Select Fee Rate with Custom") {
    AsyncPreview {
        VStack {
            SendFlowSelectFeeRateView(
                manager: WalletManager(preview: "preview_only"),
                feeOptions: Binding.constant(FeeRateOptionsWithTotalFee.previewNew().addCustomFee(feeRate: 13.77)),
                selectedOption: Binding.constant(
                    FeeRateOptionsWithTotalFee.previewNew().medium()
                ),
                selectedPresentationDetent: Binding.constant(PresentationDetent.large)
            )
            .environment(WalletManager(preview: "preview_only"))
            .environment(AppManager())
            .frame(height: 550)
        }
        .frame(maxHeight: .infinity)
        .background(.coveBg)
    }
}
