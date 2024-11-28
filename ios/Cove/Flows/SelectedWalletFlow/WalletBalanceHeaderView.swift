//
//  WalletBalanceHeaderView.swift
//  Cove
//
//  Created by Praveen Perera on 7/31/24.
//

import SwiftUI

struct WalletBalanceHeaderView: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
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

    private var primaryBalanceString: String {
        if !metadata.sensitiveVisible {
            return "************"
        }

        // fiat
        if metadata.fiatOrBtc == .fiat {
            guard let fiatAmount else { return "" }
            return model.rust.displayFiatAmount(amount: fiatAmount)
        }

        // btc or sats
        return model.amountFmtUnit(balance)
    }

    private var secondaryBalanceString: String {
        if !metadata.sensitiveVisible {
            return "************"
        }

        // fiat
        if metadata.fiatOrBtc == .btc {
            guard let fiatAmount else { return "" }
            return model.rust.displayFiatAmount(amount: fiatAmount)
        }

        // btc or sats
        return model.amountFmtUnit(balance)
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
        VStack(spacing: 26) {
            VStack(spacing: 6) {
                HStack {
                    Text(secondaryBalanceString)
                        .foregroundColor(.white.opacity(0.75))
                        .font(.footnote)
                        .padding(.leading, 2)

                    Spacer()
                }

                HStack {
                    Text(primaryBalanceString)
                        .foregroundStyle(.white)
                        .font(.system(size: fontSize, weight: .bold))

                    Spacer()

                    Image(systemName: eyeIcon)
                        .foregroundColor(.gray)
                        .onTapGesture {
                            updater(.toggleSensitiveVisibility)
                        }
                }
            }
            .contentShape(
                .contextMenuPreview,
                RoundedRectangle(cornerRadius: 8).inset(by: -8)
            )
            .contextMenu {
                Button("BTC") {
                    model.dispatch(action: .updateUnit(.btc))
                    model.dispatch(action: .updateFiatOrBtc(.btc))
                }

                Button("SATS") {
                    model.dispatch(action: .updateUnit(.sat))
                    model.dispatch(action: .updateFiatOrBtc(.btc))
                }
            }

            HStack(spacing: 16) {
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
                    .foregroundColor(Color.midnightBlue)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .padding(.vertical, 4)
                    .background(Color.btnPrimary)
                    .cornerRadius(8)
                }

                Button(action: showReceiveSheet) {
                    HStack(spacing: 10) {
                        Image(systemName: "arrow.down.left")
                        Text("Receive")
                    }
                    .foregroundColor(Color.midnightBlue)
                    .frame(maxWidth: .infinity)
                    .padding()
                    .padding(.vertical, 4)
                    .background(Color.btnPrimary)
                    .cornerRadius(8)
                }
            }
        }
        .padding()
        .padding(.vertical, 26)
        .padding(.top, safeAreaInsets.top + 25)
        .background(
            Image(.headerPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: 300, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .brightness(0.1)
        )
        .background(Color.midnightBlue.opacity(0.98))
        .onTapGesture {
            model.dispatch(action: .toggleFiatBtcPrimarySecondary)
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
            .environment(MainViewModel())
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
            .environment(MainViewModel())
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
            .environment(MainViewModel())
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
            .environment(MainViewModel())
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
            .environment(MainViewModel())
            .environment(WalletViewModel(preview: "preview_only"))
        }
}
