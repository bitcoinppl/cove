//
//  FeeBumpDelayedPopup.swift
//  Cove
//
//  Created by Praveen Perera on 11/23/26.
//

import MijickPopups
import SwiftUI

/// Explains a send that is taking longer than usual and offers to bump its fee in one tap
///
/// Popups draw in their own window, outside the app's environment, so it takes its managers as
/// arguments. It owns its `FeeBumpManager`, which loads the suggested fee as it shows
struct FeeBumpDelayedPopup: CenterPopup {
    let app: AppManager
    let walletManager: WalletManager
    let txId: TxId
    let pendingSend: PendingSendStatus

    /// Opens the bump fee sheet to pick another fee
    let customize: () -> Void

    var body: some View {
        FeeBumpDelayedPopupView(
            app: app,
            walletManager: walletManager,
            txId: txId,
            pendingSend: pendingSend,
            customize: customize
        )
        .environment(app)
        .padding(.horizontal, 8)
    }

    func configurePopup(config: Config) -> Config {
        config.tapOutsideToDismissPopup(true)
            .popupHorizontalPadding(24)
            .backgroundColor(.clear)
            .overlayColor(.black.opacity(0.4))
    }
}

private struct FeeBumpDelayedPopupView: View {
    let app: AppManager
    let walletManager: WalletManager
    let txId: TxId
    let pendingSend: PendingSendStatus
    let customize: () -> Void

    @State private var manager: FeeBumpManager?
    @State private var loadError: String?

    /// The recipient's side of the bump is known only once its options load
    private var bumpCopy: String {
        guard let options = manager?.options else { return "Speed it up with a higher fee." }
        return "Speed it up with a higher fee. \(options.recipientNote(at: manager?.selected))"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            FeeBumpDelayedPopupHeader(pendingSend: pendingSend)

            VStack(alignment: .leading, spacing: 12) {
                Text("Your network fee is lower than what miners are picking up right now, so this could take hours or longer to confirm.")
                Text(bumpCopy)
                    .accessibilityIdentifier("speedUpPopup.note")
            }
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            FeeBumpDelayedPopupComparison(manager: manager, loadError: loadError)

            FeeBumpDelayedPopupActions(
                manager: manager,
                bump: bump,
                customize: openSheet,
                dismiss: dismissPopup
            )
        }
        .padding(22)
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 28))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("speedUpPopup")
        .task { makeManager() }
        .onChange(of: manager?.readyRoute) { _, route in
            guard let route else { return }
            pushOnceClosed(route)
        }
        .modifier(FeeBumpPopupAlert(manager: manager))
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

    /// Continues straight to confirm or export at the suggested fee
    private func bump() {
        manager?.continueWithSelected()
    }

    /// The popup draws in its own window, so the push waits only for it to go
    private func pushOnceClosed(_ route: Route) {
        Task {
            await dismissLastPopup()
            await MainActor.run { app.pushRoute(route) }
        }
    }

    private func openSheet() {
        Task {
            await dismissLastPopup()
            await MainActor.run { customize() }
        }
    }

    private func dismissPopup() {
        Task { await dismissLastPopup() }
    }
}

/// The manager's alerts, shown once the manager exists
///
/// `presentingAlert` binds a manager that exists from the start, and the popup makes its own as
/// it shows
private struct FeeBumpPopupAlert: ViewModifier {
    let manager: FeeBumpManager?

    func body(content: Content) -> some View {
        content.alert(
            alertTitle,
            isPresented: isPresentingAlert,
            presenting: manager?.alert,
            actions: { alertActions($0.item) },
            message: { alertMessage($0.item) }
        )
    }

    private var isPresentingAlert: Binding<Bool> {
        Binding(
            get: { manager?.alert != nil },
            set: { if !$0 { manager?.alert = nil } }
        )
    }

    private var alertTitle: String {
        switch manager?.alert?.item {
        case let .feeWarning(warning): warning.title
        case .customRateFailed: "Unable to Use This Fee"
        case .continueFailed, nil: "Unable to Speed Up"
        }
    }

    @ViewBuilder
    private func alertActions(_ alert: FeeBumpAlert) -> some View {
        switch alert {
        case .continueFailed, .customRateFailed:
            Button("OK") { manager?.alert = nil }
        case let .feeWarning(warning):
            Button("Bump Anyway") {
                manager?.alert = nil
                manager?.acknowledge(warning)
            }

            Button("Cancel", role: .cancel) { manager?.alert = nil }
        }
    }

    private func alertMessage(_ alert: FeeBumpAlert) -> Text {
        switch alert {
        case let .continueFailed(message), let .customRateFailed(message): Text(message)
        case let .feeWarning(warning): Text(warning.message)
        }
    }
}

/// The fee comparison once it loads, or why the send can't be bumped
private struct FeeBumpDelayedPopupComparison: View {
    let manager: FeeBumpManager?
    let loadError: String?

    var body: some View {
        if let message = loadError ?? manager?.unavailableMessage {
            Text(message)
                .font(.subheadline)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("speedUpPopup.unavailable")
        } else if let options = manager?.options {
            FeeBumpComparison(options: options, suggested: manager?.selected)
        } else {
            FeeBumpLoadingView(minHeight: 70)
        }
    }
}

private struct FeeBumpDelayedPopupHeader: View {
    let pendingSend: PendingSendStatus

    var body: some View {
        HStack(alignment: .center, spacing: 14) {
            FeeBumpDelayedIcon(size: 44)

            VStack(alignment: .leading, spacing: 3) {
                Text("Taking longer than usual")
                    .font(.headline)

                TimelineView(.everyMinute) { _ in
                    Text("Unconfirmed for \(pendingSend.elapsedFmt() ?? "a while")")
                        .font(.subheadline)
                        .foregroundStyle(Color.orange)
                }
            }
        }
    }
}

/// The original fee next to the suggested one, as the popup shows them
private struct FeeBumpComparison: View {
    let options: FeeBumpOptions
    let suggested: FeeRateOptionWithTotalFee?

    var body: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Your fee")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Text("\(options.originalFee.satsString()) sats")
                    .font(.title3)
                    .fontWeight(.bold)

                Text(options.originalFeeRate.satsPerVbyteString())
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Image(systemName: "arrow.right")
                .foregroundStyle(.secondary)

            Spacer()

            VStack(alignment: .trailing, spacing: 2) {
                Text("Suggested")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Text(suggested?.totalFee().map { "\($0.satsString()) sats" } ?? "—")
                    .font(.title3)
                    .fontWeight(.bold)
                    .accessibilityIdentifier("speedUpPopup.suggestedFee")

                if let suggested {
                    Text("\(suggested.satsPerVbyteString()) · ~\(suggested.feeSpeed().duration)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(14)
        .background(Color(UIColor.systemGray6).opacity(0.8))
        .clipShape(RoundedRectangle(cornerRadius: 14))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("speedUpPopup.comparison")
    }
}

private struct FeeBumpDelayedPopupActions: View {
    let manager: FeeBumpManager?
    let bump: () -> Void
    let customize: () -> Void
    let dismiss: () -> Void

    private var isLoading: Bool {
        guard let manager else { return true }
        if case .loading = manager.status { return true }
        return manager.isContinuing
    }

    private var canBump: Bool {
        manager?.options != nil && manager?.selected != nil && !isLoading
    }

    var body: some View {
        VStack(spacing: 10) {
            Button(action: bump) {
                HStack(spacing: 8) {
                    if isLoading {
                        ProgressView()
                            .tint(.white)
                    } else {
                        Image(systemName: "bolt.fill")
                    }

                    Text("Speed Up")
                }
            }
            .buttonStyle(DarkButtonStyle())
            .disabled(!canBump)
            .accessibilityIdentifier("speedUpPopup.speedUp")

            Button(action: customize) {
                Text("Customize Fee")
            }
            .buttonStyle(DarkButtonStyle(backgroundColor: Color(UIColor.systemGray5), foregroundColor: .primary))
            .disabled(manager?.unavailableMessage != nil)
            .accessibilityIdentifier("speedUpPopup.customize")

            Button("Not now", action: dismiss)
                .font(.subheadline)
                .padding(.top, 2)
                .accessibilityIdentifier("speedUpPopup.notNow")
        }
    }
}

/// An orange clock with a `!` badge, marking a send that is taking longer than usual
struct FeeBumpDelayedIcon: View {
    var size: CGFloat = 50

    var body: some View {
        Image(systemName: "clock")
            .font(.system(size: size * 0.4, weight: .medium))
            .foregroundStyle(Color.orange)
            .frame(width: size, height: size)
            .background(Color.orange.opacity(0.15))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.orange.opacity(0.4), lineWidth: 1))
            .cornerRadius(6)
            .overlay(alignment: .topTrailing) {
                Image(systemName: "exclamationmark")
                    .font(.system(size: size * 0.2, weight: .heavy))
                    .foregroundStyle(.white)
                    .frame(width: size * 0.36, height: size * 0.36)
                    .background(Circle().fill(Color.orange))
                    .offset(x: size * 0.14, y: -size * 0.14)
            }
    }
}
