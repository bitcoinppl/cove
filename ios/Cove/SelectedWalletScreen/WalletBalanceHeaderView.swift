//
//  WalletBalanceHeaderView.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import SwiftUI

struct WalletBalanceHeaderView: View {
    @Environment(MainViewModel.self) var app
    @Environment(WalletViewModel.self) var model

    // confirmed balance
    let balance: Amount
    let metadata: WalletMetadata
    let updater: (WalletViewModelAction) -> Void
    let showReceiveSheet: () -> Void

    // private
    @State var fiatAmount: Float64? = nil

    private var accentColor: Color {
        metadata.swiftColor
    }

    private func getFiatBalance() async {
        do {
            fiatAmount = try await model.rust.balanceInFiat()
        } catch {
            Log.error("error getting fiat balance: \(error)")
        }
    }

    private var balanceString: String {
        if !metadata.sensitiveVisible {
            return "************"
        }

        // fiat
        if metadata.fiatOrBtc == .fiat {
            if let fiatAmount = fiatAmount {
                return model.rust.displayFiatAmount(amount: fiatAmount)
            } else {
                return ""
            }
        }

        // btc or sats
        return switch metadata.selectedUnit {
        case .btc: balance.btcStringWithUnit()
        case .sat: balance.satsStringWithUnit()
        }
    }

    var eyeIcon: String {
        metadata.sensitiveVisible ? "eye" : "eye.slash"
    }

    var fontSize: CGFloat {
        let btc = balance.asBtc()

        // Base font size
        let baseFontSize: CGFloat = 34

        // Calculate the number of digits
        let digits = btc > 0 ? Int(log10(btc)) + 1 : 1

        // Reduce font size by 2 for each additional digit beyond 1
        let fontSizeReduction = CGFloat(max(0, (digits - 1) * 2))

        // Ensure minimum font size of 20
        return max(baseFontSize - fontSizeReduction, 20)
    }

    var body: some View {
        VStack {
            HStack {
                Picker(
                    "Currency",
                    selection: Binding(
                        get: { metadata.selectedUnit },
                        set: { updater(.updateUnit($0)) }
                    )
                ) {
                    Text(String(Unit.btc)).tag(Unit.btc)
                    Text(String(Unit.sat)).tag(Unit.sat)
                }
                .pickerStyle(SegmentedPickerStyle())
                .frame(width: 120)

                Spacer()

                Image(systemName: eyeIcon)
                    .foregroundColor(.gray)
                    .onTapGesture {
                        updater(.toggleSensitiveVisibility)
                    }
            }

            HStack {
                Text("Your Balance")
                    .foregroundColor(.gray)
                    .font(.subheadline)
                    .padding(.leading, 2)

                Spacer()
            }

            Text(balanceString)
                .font(.system(size: fontSize, weight: .bold))
                .padding(.top, 16)
                .padding(.bottom, 32)

            HStack(spacing: 16) {
                Button(action: showReceiveSheet) {
                    HStack(spacing: 10) {
                        Image(systemName: "arrow.down.left")
                        Text("Receive")
                    }
                    .foregroundColor(.white)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(accentColor)
                    .cornerRadius(8)
                }

                Button(action: {
                    if balance.asSats() == 0 {
                        model.errorAlert = .noBalance
                        return
                    }
                    
                    app.pushRoute(RouteFactory().sendSetAmount(id: metadata.id))
                }) {
                    HStack(spacing: 10) {
                        Image(systemName: "arrow.up.right")

                        Text("Send")
                    }
                    .foregroundColor(accentColor)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .background(Color.white)
                    .cornerRadius(8)
                    .overlay(
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(accentColor, lineWidth: 1)
                    )
                }
            }
        }
        .padding()
        .background(Color(UIColor.systemGray6))
        .onTapGesture {
            model.dispatch(action: .toggleFiatOrBtc)
        }
        .task {
            await getFiatBalance()
        }
    }
}

#Preview("btc") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance: Amount.fromSat(sats: 1_000_738),
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .padding()
            .environment(WalletViewModel(preview: "preview_only"))
        }
}

#Preview("sats") {
    var metadata = walletMetadataPreview()
    metadata.selectedUnit = .sat
    metadata.sensitiveVisible = true
    metadata.color = .blue

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance: Amount.fromSat(sats: 1_000_738),
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .padding()
            .environment(WalletViewModel(preview: "preview_only"))
        }
}

#Preview("hidden") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = false
    metadata.color = .green

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance:
                Amount.fromSat(sats: 1_000_738),
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .padding()
            .environment(WalletViewModel(preview: "preview_only"))
        }
}

#Preview("lots of btc") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true
    metadata.color = .purple

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance:
                Amount.fromSat(sats: 10_000_000_738),
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .padding()
            .environment(WalletViewModel(preview: "preview_only"))
        }
}

#Preview("in fiat") {
    var metadata = walletMetadataPreview()
    metadata.sensitiveVisible = true
    metadata.color = .purple
    metadata.fiatOrBtc = .fiat

    return
        AsyncPreview {
            WalletBalanceHeaderView(
                balance:
                Amount.fromSat(sats: 10_000_000_738),
                metadata: metadata,
                updater: { _ in () },
                showReceiveSheet: {}
            )
            .padding()
            .environment(WalletViewModel(preview: "preview_only"))
        }
}
