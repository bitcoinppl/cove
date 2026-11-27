//
//  SendFlowConfirmScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

private let sendConfirmationActionTopPadding: CGFloat = 20
private let sendConfirmationActionBottomPadding: CGFloat = 8

struct SendFlowConfirmScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(SendFlowPresenter.self) private var presenter

    let id: WalletId
    @State var manager: WalletManager
    let details: ConfirmDetails
    let input: SendConfirmationInput
    let payjoinEndpoint: String?

    let prices: PriceResponse? = nil

    // private
    @State private var finalizedTransaction: BitcoinTransaction? = nil
    @State private var sendState: SendState = .idle

    /// popover to change btc and sats
    @State private var showingMenu: Bool = false

    /// locking task, cancel if its screen is leaving
    @State private var lockingTask: Task<Void, Never>? = nil

    /// Lock this screen took to ask for authentication before sending
    @State private var sendConfirmationLock: SendConfirmationLock? = nil

    /// The pending send this transaction replaces when it is a fee bump, asked for as the screen
    /// opens and before sending at the latest, since after the broadcast nothing is left for it
    /// to replace
    @State private var feeBump: FeeBumpCheck = .checking

    /// The fee bump's replacement once broadcast, which this screen then shows as Sped Up
    @State private var spedUp: FeeBumpBroadcast?

    var fiatAmount: String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = details.sendingAmount()
        return manager.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    /// The send result waits for any app lock to clear, so a send that finishes while the app
    /// is away never shows its result above the lock screen
    private var resultAlertBinding: Binding<TaggedItem<SendFlowConfirmAlertState>?> {
        Binding(
            get: { auth.lockState == .unlocked ? presenter.confirmationAlertState : nil },
            set: { newValue in
                // the user can't act on a hidden alert, so a write while locked is the lock
                // hiding it and must keep the result for after unlock
                guard auth.lockState == .unlocked else { return }
                presenter.confirmationAlertState = newValue
            }
        )
    }

    private var confirmationAlertContext: SendFlowConfirmAlertContext {
        SendFlowConfirmAlertContext(presenter: presenter, sendState: $sendState)
    }

    var body: some View {
        if let spedUp {
            FeeBumpedScreen(broadcast: spedUp, manager: manager)
        } else if case let .signedPsbt(psbt) = input, finalizedTransaction == nil {
            SendFlowFinalizePsbtView(
                manager: manager,
                psbt: psbt,
                finalizedTransaction: $finalizedTransaction
            )
        } else {
            SendFlowConfirmationContent(
                manager: manager,
                details: details,
                feeBump: feeBump.context,
                prices: prices,
                fiatAmount: fiatAmount,
                showingMenu: $showingMenu,
                sendState: $sendState,
                lockingTask: $lockingTask,
                walletType: metadata.walletType,
                confirmSend: confirmSend,
                lockWallet: lockForSendConfirmation
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.coveBg)
            // nothing here takes text, and the amount screen's keyboard and its toolbar must not
            // lift the swipe control while they leave
            .ignoresSafeArea(.keyboard)
            // the screen shows as a plain send until the answer comes, a few milliseconds later,
            // so no send waits behind a spinner
            .task { await checkFeeBump() }
            .onDisappear(perform: handleDisappear)
            .onChange(of: manager.payjoinTxBroadcast, payjoinBroadcastChanged)
            .onChange(of: manager.sendFlowErrorAlert, sendFlowErrorChanged)
            .presentingAlert(
                resultAlertBinding,
                context: confirmationAlertContext,
                defaultTitle: "Error Broadcasting!"
            )
        }
    }

    /// Asks once, keeping the first answer, since one that arrives after the broadcast would
    /// find nothing left to replace
    private func checkFeeBump() async {
        guard case .checking = feeBump else { return }

        let context = await manager.feeBumpContext(details: details)
        guard case .checking = feeBump else { return }

        feeBump = .checked(context)
    }

    /// A fee bump goes on to the Sped Up screen, any other send shows the sent alert
    private func sent(txId: TxId) {
        sendState = .sent

        guard feeBump.context != nil else {
            presenter.confirmationAlertState = .init(.sent(id))
            return
        }

        withAnimation { spedUp = feeBumpBroadcast(txId: txId, details: details) }
    }

    private func confirmSend() {
        sendState = .sending
        Task { await sendTransaction() }
    }

    private func lockForSendConfirmation() {
        sendConfirmationLock = auth.lockForSendConfirmation()
    }

    /// Gives back only this screen's lock, any other lock still needs authentication
    private func handleDisappear() {
        lockingTask?.cancel()
        guard let sendConfirmationLock else { return }

        auth.releaseSendConfirmationLock(sendConfirmationLock)
        self.sendConfirmationLock = nil
    }

    private func payjoinBroadcastChanged(_: UUID?, _ uuid: UUID?) {
        // UUID changes each time so this fires reliably across multiple sends
        guard uuid != nil, case .sending = sendState else { return }

        sendState = .sent
        presenter.confirmationAlertState = .init(.sent(id))
    }

    private func sendFlowErrorChanged(
        _: TaggedItem<SendFlowErrorAlert>?,
        _ alert: TaggedItem<SendFlowErrorAlert>?
    ) {
        // payjoin broadcast failure arrives via reconcile, so unblock the sending UI here
        guard let alert, case .sending = sendState else { return }

        let errorMessage =
            switch alert.item {
            case let .signAndBroadcast(error): error
            case let .confirmDetails(error): error
            }

        sendState = .error(errorMessage)
        manager.sendFlowErrorAlert = nil
        presenter.confirmationAlertState = .init(.broadcastError(errorMessage))
    }

    @MainActor
    private func sendTransaction() async {
        // a swipe that beats the check still needs its answer, which the broadcast would erase
        await checkFeeBump()

        do {
            switch input {
            case let .signedTransaction(txn):
                try await manager.broadcastTransaction(txn)
                sent(txId: txn.txId())
            case .signedPsbt:
                guard let finalizedTransaction else {
                    throw SendConfirmationError.unfinalizedSignedPsbt
                }
                try await manager.broadcastTransaction(finalizedTransaction)
                sent(txId: finalizedTransaction.txId())
            case .unsigned:
                try await manager.initiatePayment(
                    psbt: details.psbt(),
                    payjoinEndpoint: payjoinEndpoint
                )
                // for payjoin, stay in .sending until PayjoinTxBroadcast reconciles
                if payjoinEndpoint == nil {
                    sent(txId: details.id())
                }
            }
        } catch let error as WalletManagerError {
            sendState = .error(error.description)
            presenter.confirmationAlertState = .init(.broadcastError(error.description))
        } catch {
            sendState = .error(error.localizedDescription)
            presenter.confirmationAlertState = .init(
                .broadcastError(error.localizedDescription)
            )
        }
    }
}

private struct SendFlowFinalizePsbtView: View {
    @Environment(AppManager.self) private var app

    let manager: WalletManager
    let psbt: Psbt
    @Binding var finalizedTransaction: BitcoinTransaction?

    var body: some View {
        FullPageLoadingView()
            .task { await finalizePsbt() }
    }

    private func finalizePsbt() async {
        do {
            finalizedTransaction = try await manager.finalizePsbt(psbt)
        } catch let error as WalletManagerError {
            app.alertState = .init(.general(
                title: "Unable to finalize transaction",
                message: error.description
            ))
        } catch {
            app.alertState = .init(.general(
                title: "Unknown error",
                message: error.localizedDescription
            ))
        }
    }
}

private struct SendFlowConfirmationContent: View {
    @Bindable var manager: WalletManager

    let details: ConfirmDetails
    let feeBump: FeeBumpConfirmContext?
    let prices: PriceResponse?
    let fiatAmount: String
    @Binding var showingMenu: Bool
    @Binding var sendState: SendState
    @Binding var lockingTask: Task<Void, Never>?
    let walletType: WalletType
    let confirmSend: () -> Void
    let lockWallet: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

            GeometryReader { geometry in
                SendFlowConfirmationBody(
                    manager: manager,
                    details: details,
                    feeBump: feeBump,
                    prices: prices,
                    fiatAmount: fiatAmount,
                    showingMenu: $showingMenu,
                    sendState: $sendState,
                    lockingTask: $lockingTask,
                    walletType: walletType,
                    confirmSend: confirmSend,
                    lockWallet: lockWallet,
                    size: geometry.size
                )
            }
        }
    }
}

private struct SendFlowConfirmationBody: View {
    @Bindable var manager: WalletManager

    let details: ConfirmDetails
    let feeBump: FeeBumpConfirmContext?
    let prices: PriceResponse?
    let fiatAmount: String
    @Binding var showingMenu: Bool
    @Binding var sendState: SendState
    @Binding var lockingTask: Task<Void, Never>?
    let walletType: WalletType
    let confirmSend: () -> Void
    let lockWallet: () -> Void
    let size: CGSize

    var body: some View {
        // the swipe control scrolls with the details, and filling the visible height keeps it at
        // the bottom of the screen whenever the details are short enough to fit
        ScrollView {
            FillHeightLayout(minHeight: size.height) {
                content
            }
        }
        .scrollIndicators(.hidden)
        .background(Color.coveBg)
    }

    private var content: some View {
        VStack(spacing: 0) {
            VStack(spacing: 24) {
                SendFlowConfirmationSummary(
                    manager: manager,
                    details: details,
                    isFeeBump: feeBump != nil,
                    fiatAmount: fiatAmount,
                    showingMenu: $showingMenu
                )

                SendFlowAccountSection(manager: manager)
                    .padding(.top)

                Divider()

                SendFlowDetailsView(manager: manager, details: details, prices: prices, feeBump: feeBump)
            }

            Spacer(minLength: 0)

            SendFlowConfirmationFooter(
                sendState: $sendState,
                lockingTask: $lockingTask,
                swipePrompt: feeBump == nil ? "Swipe to Send" : "Swipe to confirm new fee",
                walletType: walletType,
                confirmSend: confirmSend,
                lockWallet: lockWallet
            )
        }
        .padding(.horizontal)
    }
}

private struct SendFlowConfirmationSummary: View {
    @Bindable var manager: WalletManager

    let details: ConfirmDetails
    let isFeeBump: Bool
    let fiatAmount: String
    @Binding var showingMenu: Bool

    var body: some View {
        VStack(spacing: 24) {
            SendFlowConfirmationTitle(isFeeBump: isFeeBump)
                .padding(.top, 10)

            SendFlowConfirmationAmount(
                amount: manager.amountFmt(details.sendingAmount()),
                unit: manager.walletMetadata.selectedUnit,
                fiatAmount: fiatAmount,
                showingMenu: $showingMenu,
                selectSats: selectSats,
                selectBtc: selectBtc
            )
        }
    }

    private func selectSats() {
        manager.dispatch(action: .updateUnit(.sat))
        showingMenu = false
    }

    private func selectBtc() {
        manager.dispatch(action: .updateUnit(.btc))
        showingMenu = false
    }
}

private struct SendFlowConfirmationTitle: View {
    let isFeeBump: Bool

    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Text(isFeeBump ? "Speed Up" : "You're sending")
                    .font(.headline)
                    .fontWeight(.bold)
                    .accessibilityIdentifier("sendFlowConfirm.title")

                Spacer()
            }
            .padding(.top, 6)

            HStack {
                Text("The amount they will receive")
                    .font(.footnote)
                    .foregroundStyle(.secondary.opacity(0.80))
                    .fontWeight(.medium)
                Spacer()
            }
        }
    }
}

private struct SendFlowConfirmationAmount: View {
    let amount: String
    let unit: Unit
    let fiatAmount: String
    @Binding var showingMenu: Bool
    let selectSats: () -> Void
    let selectBtc: () -> Void

    var body: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom) {
                Spacer()

                Text(amount)
                    .frame(minWidth: screenWidth / 2)
                    .font(.system(size: 48, weight: .bold))
                    .minimumScaleFactor(0.01)
                    .lineLimit(1)
                    .multilineTextAlignment(.center)

                SendFlowConfirmationUnitSelector(
                    unit: unit,
                    showingMenu: $showingMenu,
                    selectSats: selectSats,
                    selectBtc: selectBtc
                )
            }
            .frame(alignment: .center)

            Text(fiatAmount)
                .font(.title3)
                .foregroundColor(.secondary)
        }
        .padding(.top, 8)
    }
}

private struct SendFlowConfirmationUnitSelector: View {
    let unit: Unit
    @Binding var showingMenu: Bool
    let selectSats: () -> Void
    let selectBtc: () -> Void

    var body: some View {
        Button(action: toggleMenu) {
            HStack(spacing: 0) {
                Text(unit == .sat ? "sats" : "btc")

                Image(systemName: "chevron.down")
                    .font(.caption)
                    .fontWeight(.bold)
                    .padding(.top, 2)
                    .padding(.leading, 4)
            }
            .frame(alignment: .trailing)
        }
        .foregroundStyle(.primary)
        .padding(.vertical, 10)
        .padding(.leading, 16)
        .popover(isPresented: $showingMenu) {
            SendFlowConfirmationUnitMenu(
                selectSats: selectSats,
                selectBtc: selectBtc
            )
        }
    }

    private func toggleMenu() {
        showingMenu.toggle()
    }
}

private struct SendFlowConfirmationUnitMenu: View {
    let selectSats: () -> Void
    let selectBtc: () -> Void

    var body: some View {
        VStack(alignment: .center, spacing: 0) {
            Button("sats", action: selectSats)
                .padding(12)
                .buttonStyle(.plain)

            Divider()

            Button("btc", action: selectBtc)
                .padding(12)
                .buttonStyle(.plain)
        }
        .padding(.vertical, 8)
        .padding(.horizontal, 12)
        .frame(minWidth: 120, maxWidth: 200)
        .presentationCompactAdaptation(.popover)
        .foregroundStyle(.primary.opacity(0.8))
    }
}

private struct SendFlowConfirmationFooter: View {
    @Binding var sendState: SendState
    @Binding var lockingTask: Task<Void, Never>?

    let swipePrompt: String

    let walletType: WalletType
    let confirmSend: () -> Void
    let lockWallet: () -> Void

    var body: some View {
        SwipeToSendView(
            sendState: $sendState,
            prompt: swipePrompt,
            onConfirm: confirmSend
        )
        .padding(.top, sendConfirmationActionTopPadding)
        .padding(.bottom, sendConfirmationActionBottomPadding)
        .onAppear(perform: lockAfterAppearance)
    }

    private func lockAfterAppearance() {
        lockingTask = Task {
            try? await Task.sleep(for: .milliseconds(50))
            if Task.isCancelled { return }

            if walletType == .hot { lockWallet() }
        }
    }
}

/// Whether the transaction replaces a pending send, which takes a moment to learn
private enum FeeBumpCheck {
    case checking
    case checked(FeeBumpConfirmContext?)

    var context: FeeBumpConfirmContext? {
        if case let .checked(context) = self { return context }
        return nil
    }
}

private enum SendConfirmationError: LocalizedError {
    case unfinalizedSignedPsbt

    var errorDescription: String? {
        "Unable to finalize transaction"
    }
}

#if DEBUG
    #Preview {
        struct Container: View {
            @State private var metadata: WalletMetadata
            @State private var manager: WalletManager?

            init() {
                var metadata = WalletMetadata(preview: true)
                metadata.selectedUnit = .sat

                self.metadata = metadata
                self.manager = nil
            }

            var body: some View {
                NavigationStack {
                    AsyncPreview {
                        Group {
                            if let manager {
                                SendFlowConfirmScreen(
                                    id: WalletId(),
                                    manager: manager,
                                    details: confirmDetailsPreviewNew(),
                                    input: .unsigned,
                                    payjoinEndpoint: nil
                                )
                                .environment(AppManager.shared)
                                .environment(AuthManager.shared)
                                .environment(
                                    SendFlowPresenter(app: AppManager.shared, manager: manager)
                                )
                            }
                        }
                    }
                    .task {
                        manager = WalletManager(preview: .only, metadata)
                        manager?.dispatch(action: .updateUnit(.sat))
                    }
                }
            }
        }

        return Container()
    }
#endif

#Preview("large") {
    AsyncPreview {
        SendFlowConfirmScreen(
            id: WalletId(),
            manager: WalletManager(preview: .only),
            details: confirmDetailsPreviewNew(),
            input: .unsigned,
            payjoinEndpoint: nil
        )
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
        .environment(
            SendFlowPresenter(
                app: AppManager.shared,
                manager: WalletManager(preview: .only)
            )
        )
    }
}
