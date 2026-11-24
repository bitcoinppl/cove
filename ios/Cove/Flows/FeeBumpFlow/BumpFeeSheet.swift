//
//  BumpFeeSheet.swift
//  Cove
//
//  Created by Praveen Perera on 11/23/26.
//

import SwiftUI

/// The pending send a bump fee sheet opens for, as a sheet item
struct FeeBumpTarget: Identifiable, Equatable {
    let txId: TxId

    var id: String {
        txId.description
    }

    static func == (lhs: FeeBumpTarget, rhs: FeeBumpTarget) -> Bool {
        lhs.id == rhs.id
    }
}

/// Picks the fee a pending send's replacement pays, then moves on to confirm or export it
///
/// Owns its `FeeBumpManager`, made when the sheet shows, so every sheet loads fresh rates. Present
/// it with `feeBumpSheet(target:walletManager:onDismiss:)`, which pushes the route Continue built
/// once the sheet is gone
struct BumpFeeSheet: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss

    let walletManager: WalletManager
    let txId: TxId

    /// Continue built the replacement, and the presenter closes the sheet and pushes this route
    let onReady: (Route) -> Void

    @State private var manager: FeeBumpManager?
    @State private var loadError: String?

    var body: some View {
        Group {
            if let manager {
                BumpFeeSheetContent(manager: manager, walletManager: walletManager, close: close)
            } else if let loadError {
                FeeBumpUnavailableView(message: loadError, close: close)
            } else {
                FeeBumpLoadingView()
            }
        }
        .background(Color.coveBg)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("speedUp.sheet")
        .task { makeManager() }
        .onChange(of: manager?.readyRoute) { _, route in
            guard let route else { return }
            onReady(route)
        }
        .onChange(of: app.router.routes) { _, _ in close() }
    }

    private func makeManager() {
        guard manager == nil else { return }

        do {
            manager = try walletManager.newFeeBumpManager(txId: txId)
        } catch {
            Log.error("Unable to open fee bump for \(txId): \(error)")
            loadError = "Unable to open this transaction. Please try again."
        }
    }

    private func close() {
        dismiss()
    }
}

private struct BumpFeeSheetContent: View {
    @Environment(AppManager.self) private var app

    @Bindable var manager: FeeBumpManager
    let walletManager: WalletManager
    let close: () -> Void

    @State private var isPresentingCustomRate = false

    private var fiat: FeeBumpFiat {
        FeeBumpFiat(app: app, manager: walletManager)
    }

    var body: some View {
        VStack(spacing: 0) {
            FeeBumpSheetCloseButton(action: close)

            switch manager.status {
            case .loading:
                FeeBumpLoadingView()
            case let .unavailable(unavailable):
                FeeBumpUnavailableView(message: unavailable.userMessage(), close: close)
            case let .ready(options):
                BumpFeeSheetReady(
                    manager: manager,
                    options: options,
                    fiat: fiat,
                    customize: { isPresentingCustomRate = true }
                )
            }
        }
        .onAppear(perform: manager.screenAppeared)
        .presentingAlert($manager.alert, context: manager, defaultTitle: "Unable to Speed Up")
        .sheet(isPresented: $isPresentingCustomRate) {
            if let options = manager.options {
                BumpFeeCustomRateSheet(manager: manager, walletManager: walletManager, options: options)
                    .presentationDetents([.height(330)])
            }
        }
    }
}

/// The rates to pick from and what the pick costs, once the sheet has loaded them
private struct BumpFeeSheetReady: View {
    let manager: FeeBumpManager
    let options: FeeBumpOptions
    let fiat: FeeBumpFiat
    let customize: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                BumpFeeSheetHeader(options: options)

                Divider()

                BumpFeeOptionList(
                    manager: manager,
                    options: options,
                    fiat: fiat,
                    customize: customize
                )

                if let selected = manager.selected, let total = selected.totalFee() {
                    BumpFeeCostSummary(newFee: total, originalFee: options.originalFee, fiat: fiat)
                }

                Text("Replaces your pending transaction with the same one at a higher fee (RBF). \(options.recipientNote(at: manager.selected))")
                    .accessibilityIdentifier("speedUp.note")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                BumpFeeContinueButton(manager: manager)
                    .padding(.top, 4)
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 20)
        }
        .scrollIndicators(.hidden)
    }
}

private struct BumpFeeSheetHeader: View {
    let options: FeeBumpOptions

    private var currentSpeed: FeeSpeed {
        options.feeOptions.calculateCustomFeeSpeed(feeRate: options.originalFeeRate.satPerVb())
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Speed Up")
                .font(.title2)
                .fontWeight(.bold)

            Text("Current fee")
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.top, 4)

            HStack(spacing: 10) {
                Text("\(options.originalFee.satsString()) sats · \(options.originalFeeRate.satsPerVbyteString())")
                    .font(.subheadline)
                    .fontWeight(.semibold)
                    .accessibilityIdentifier("speedUp.currentFee")

                FeeBumpDurationChip(text: currentSpeed.duration)
            }
        }
    }
}

/// A plain gray chip with how long a rate should take to confirm
struct FeeBumpDurationChip: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption2)
            .fontWeight(.semibold)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(Color.gray.opacity(0.2))
            .cornerRadius(6)
    }
}

private struct BumpFeeOptionList: View {
    let manager: FeeBumpManager
    let options: FeeBumpOptions
    let fiat: FeeBumpFiat
    let customize: () -> Void

    private var feeOptions: FeeRateOptionsWithTotalFee {
        options.feeOptions
    }

    var body: some View {
        VStack(spacing: 12) {
            row(feeOptions.fast())
            row(feeOptions.medium())
            row(feeOptions.slow())

            if let custom = feeOptions.custom() {
                row(custom, onSelect: customize)
            } else {
                BumpFeeCustomPlaceholderRow(action: customize)
            }
        }
    }

    private func row(_ option: FeeRateOptionWithTotalFee, onSelect: (() -> Void)? = nil) -> some View {
        FeeRateOptionRow(
            feeOption: option,
            isSelected: manager.selected?.feeSpeed() == option.feeSpeed(),
            fiatAmount: option.totalFee().flatMap(fiat.approximate),
            missingTotalText: "Not enough funds",
            identifierPrefix: "speedUp.option",
            onSelect: onSelect ?? { manager.select(option) }
        )
    }
}

private struct BumpFeeCustomPlaceholderRow: View {
    let action: () -> Void

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text("Custom")
                    .font(.headline)

                Text("Set your own fee rate")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }

            Spacer()
        }
        .padding()
        .background(Color(UIColor.systemGray6))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.secondary.opacity(0.5), lineWidth: 1))
        .cornerRadius(12)
        .contentShape(Rectangle())
        .onTapGesture(perform: action)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("speedUp.option.custom")
    }
}

private struct BumpFeeCostSummary: View {
    let newFee: Amount
    let originalFee: Amount
    let fiat: FeeBumpFiat

    private var additional: Amount {
        Amount.fromSat(sats: newFee.asSats() - min(originalFee.asSats(), newFee.asSats()))
    }

    var body: some View {
        VStack(spacing: 6) {
            row("New network fee", sats: "\(newFee.satsString()) sats", amount: newFee)
                .accessibilityIdentifier("speedUp.newFee")

            row("Additional cost", sats: "+\(additional.satsString()) sats", amount: additional)
                .accessibilityIdentifier("speedUp.additionalCost")
        }
        .font(.subheadline)
    }

    private func row(_ title: String, sats: String, amount: Amount) -> some View {
        HStack {
            Text(title)
                .foregroundStyle(.secondary)

            Spacer()

            Text(sats)
                .fontWeight(.semibold)

            if let fiatAmount = fiat.approximate(amount) {
                Text(fiatAmount)
                    .fontWeight(.semibold)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

private struct BumpFeeContinueButton: View {
    let manager: FeeBumpManager

    private var title: String {
        guard let total = manager.selected?.totalFee() else { return "Speed Up" }
        return "Speed Up for \(total.satsString()) sats"
    }

    var body: some View {
        Button(action: manager.continueWithSelected) {
            HStack(spacing: 8) {
                if manager.isContinuing {
                    ProgressView()
                        .tint(.white)
                } else {
                    Image(systemName: "bolt.fill")
                }

                Text(title)
            }
        }
        .buttonStyle(DarkButtonStyle())
        .disabled(manager.isContinuing || manager.selected == nil)
        .accessibilityIdentifier("speedUp.continue")
    }
}

struct FeeBumpSheetCloseButton: View {
    let action: () -> Void

    var body: some View {
        HStack {
            Spacer()

            Button(action: action) {
                Image(systemName: "xmark")
                    .font(.footnote.weight(.bold))
                    .foregroundStyle(.secondary)
                    .frame(width: 32, height: 32)
                    .background(Color(UIColor.systemGray5))
                    .clipShape(Circle())
            }
            .accessibilityLabel("Close")
            .accessibilityIdentifier("speedUp.close")
        }
        .padding(.horizontal, 16)
        .padding(.top, 16)
    }
}

struct FeeBumpLoadingView: View {
    var minHeight: CGFloat = 200

    var body: some View {
        ProgressView()
            .tint(.primary)
            .frame(maxWidth: .infinity, minHeight: minHeight)
            .accessibilityIdentifier("speedUp.loading")
    }
}

struct FeeBumpUnavailableView: View {
    let message: String
    let close: () -> Void

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "exclamationmark.circle")
                .font(.largeTitle)
                .foregroundStyle(.secondary)

            Text(message)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("speedUp.unavailable")

            Button("Close", action: close)
                .buttonStyle(DarkButtonStyle())
        }
        .padding(24)
    }
}

extension FeeBumpOptions {
    /// What a bump at `option` changes for the recipient, as the fee bump screens say it
    func recipientNote(at option: FeeRateOptionWithTotalFee?) -> String {
        guard case .fromAmount = payment else { return "The amount and recipient stay the same." }
        guard let option, let less = payment.amountTaken(option: option) else {
            return "The recipient receives less, by the extra fee."
        }

        return "The recipient receives \(less.satsString()) sats less."
    }
}

extension FeeBumpAlert: TaggedAlertPresentable {
    func alert(context manager: FeeBumpManager) -> AnyAlertBuilder {
        switch self {
        case let .continueFailed(message):
            AlertBuilder(
                title: "Unable to Speed Up",
                message: message,
                actions: { Button("OK") { manager.alert = nil } }
            ).eraseToAny()

        case let .customRateFailed(message):
            AlertBuilder(
                title: "Unable to Use This Fee",
                message: message,
                actions: { Button("OK") { manager.alert = nil } }
            ).eraseToAny()

        case let .feeWarning(warning):
            AlertBuilder(
                title: warning.title,
                message: warning.message,
                actions: {
                    Button("Bump Anyway") {
                        manager.alert = nil
                        manager.acknowledge(warning)
                    }

                    Button("Cancel", role: .cancel) { manager.alert = nil }
                }
            ).eraseToAny()
        }
    }
}

extension View {
    /// Presents the bump fee sheet for `target`, and opens it from any `openFeeBump` below
    ///
    /// The route Continue builds is pushed only from the sheet's `onDismiss`: a push that animates
    /// while the sheet is still sliding away can leave the navigation stack out of step with the
    /// router, and then it shows no later push, such as the Sped Up screen
    func feeBumpSheet(
        target: Binding<FeeBumpTarget?>,
        walletManager: WalletManager,
        onDismiss: @escaping () -> Void = {}
    ) -> some View {
        modifier(FeeBumpSheetPresenter(target: target, walletManager: walletManager, onDismiss: onDismiss))
    }
}

private struct FeeBumpSheetPresenter: ViewModifier {
    @Environment(AppManager.self) private var app

    @Binding var target: FeeBumpTarget?
    let walletManager: WalletManager
    let onDismiss: () -> Void

    @State private var readyRoute: Route?

    func body(content: Content) -> some View {
        content
            .environment(\.openFeeBump) { target = FeeBumpTarget(txId: $0) }
            .sheet(item: $target, onDismiss: dismissed) { target in
                BumpFeeSheet(walletManager: walletManager, txId: target.txId, onReady: ready)
                    .presentationDetents([.large])
            }
    }

    private func ready(_ route: Route) {
        readyRoute = route
        target = nil
    }

    private func dismissed() {
        onDismiss()

        guard let route = readyRoute else { return }
        readyRoute = nil
        app.pushRoute(route)
    }
}
