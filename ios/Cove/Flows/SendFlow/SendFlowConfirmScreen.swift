//
//  SendFlowConfirmScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

private let sendConfirmationActionTopPadding: CGFloat = 20
private let sendConfirmationActionBottomPadding: CGFloat = 24

struct SendFlowConfirmScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(SendFlowPresenter.self) private var presenter

    let id: WalletId
    @State var manager: WalletManager
    let details: ConfirmDetails
    let input: SendConfirmationInput

    let prices: PriceResponse? = nil

    // private
    @State private var finalizedTransaction: BitcoinTransaction? = nil
    @State private var sendState: SendState = .idle

    @State private var sendConfirmationFooterHeight: CGFloat = 0

    /// popover to change btc and sats
    @State private var showingMenu: Bool = false

    /// locking task, cancel if its screen is leaving
    @State private var lockingTask: Task<Void, Never>? = nil

    /// Lock this screen took to ask for authentication before sending
    @State private var sendConfirmationLock: SendConfirmationLock? = nil

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

    /// The Payjoin session this screen starts, nil for an ordinary send
    private var payjoinSessionId: PayjoinSessionId? {
        guard case let .unsigned(mode: .payjoin(intent: intent)) = input else { return nil }
        return intent.sessionId
    }

    /// The wallet's Payjoin status when it is this screen's payment; another payment's
    /// status never changes this screen
    private var ownPayjoinStatus: PayjoinStatus? {
        guard let payjoinSessionId, manager.payjoinStatus.sessionId == payjoinSessionId
        else { return nil }

        return manager.payjoinStatus
    }

    private var confirmationAlertContext: SendFlowConfirmAlertContext {
        SendFlowConfirmAlertContext(presenter: presenter, sendState: $sendState)
    }

    var body: some View {
        if case let .signedPsbt(psbt) = input, finalizedTransaction == nil {
            SendFlowFinalizePsbtView(
                manager: manager,
                psbt: psbt,
                finalizedTransaction: $finalizedTransaction
            )
        } else {
            SendFlowConfirmationContent(
                manager: manager,
                details: details,
                prices: prices,
                fiatAmount: fiatAmount,
                showingMenu: $showingMenu,
                sendState: $sendState,
                footerHeight: $sendConfirmationFooterHeight,
                lockingTask: $lockingTask,
                walletType: metadata.walletType,
                payjoinWaiting: ownPayjoinStatus?.negotiation,
                confirmSend: confirmSend,
                lockWallet: lockForSendConfirmation,
                sendWithoutPayjoin: sendWithoutPayjoin
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.coveBg)
            .onDisappear(perform: handleDisappear)
            // initial, so a status the wallet already holds for this session applies on appear
            .onChange(of: manager.payjoinStatus, initial: true, payjoinStatusChanged)
            .onChange(of: manager.sendFlowErrorAlert, sendFlowErrorChanged)
            .presentingAlert(
                resultAlertBinding,
                context: confirmationAlertContext,
                defaultTitle: "Error Broadcasting!"
            )
        }
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

    /// Cancels only the wait for the receiver; the wallet broadcasts the normal transaction at once
    private func sendWithoutPayjoin(_ sessionId: PayjoinSessionId) {
        manager.dispatch(action: .cancelPayjoin(sessionId))
    }

    private func payjoinStatusChanged(_: PayjoinStatus, _: PayjoinStatus) {
        guard let status = ownPayjoinStatus else { return }

        switch status {
        // the session id is new to this screen, so a status naming it means this payment started
        case .negotiating, .broadcasting:
            if case .idle = sendState { sendState = .sending }

        // a fallback is a normal send, not an error
        case let .completed(sessionId: _, txid: _, kind: kind):
            if case .sent = sendState { return }

            sendState = .sent
            presenter.confirmationAlertState = switch kind {
            case .proposal: .init(.sent(id))
            case .fallback: .init(.sentWithoutPayjoin(id))
            }

        case .idle:
            break
        }
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
        do {
            switch input {
            case let .signedTransaction(txn):
                try await manager.broadcastTransaction(txn)
            case .signedPsbt:
                guard let finalizedTransaction else {
                    throw SendConfirmationError.unfinalizedSignedPsbt
                }
                try await manager.broadcastTransaction(finalizedTransaction)
            case let .unsigned(mode: mode):
                try await manager.initiatePayment(psbt: details.psbt(), mode: mode)
                // for payjoin, stay in .sending until its session completes
                if case .standard = mode {
                    sendState = .sent
                    presenter.confirmationAlertState = .init(.sent(id))
                }
                return
            }
            sendState = .sent
            presenter.confirmationAlertState = .init(.sent(id))
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
    let prices: PriceResponse?
    let fiatAmount: String
    @Binding var showingMenu: Bool
    @Binding var sendState: SendState
    @Binding var footerHeight: CGFloat
    @Binding var lockingTask: Task<Void, Never>?
    let walletType: WalletType
    let payjoinWaiting: PayjoinNegotiation?
    let confirmSend: () -> Void
    let lockWallet: () -> Void
    let sendWithoutPayjoin: (PayjoinSessionId) -> Void

    var body: some View {
        VStack(spacing: 0) {
            SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

            GeometryReader { geometry in
                SendFlowConfirmationBody(
                    manager: manager,
                    details: details,
                    prices: prices,
                    fiatAmount: fiatAmount,
                    showingMenu: $showingMenu,
                    sendState: $sendState,
                    footerHeight: $footerHeight,
                    lockingTask: $lockingTask,
                    walletType: walletType,
                    payjoinWaiting: payjoinWaiting,
                    confirmSend: confirmSend,
                    lockWallet: lockWallet,
                    sendWithoutPayjoin: sendWithoutPayjoin,
                    size: geometry.size
                )
            }
        }
    }
}

private struct SendFlowConfirmationBody: View {
    @Bindable var manager: WalletManager

    let details: ConfirmDetails
    let prices: PriceResponse?
    let fiatAmount: String
    @Binding var showingMenu: Bool
    @Binding var sendState: SendState
    @Binding var footerHeight: CGFloat
    @Binding var lockingTask: Task<Void, Never>?
    let walletType: WalletType
    let payjoinWaiting: PayjoinNegotiation?
    let confirmSend: () -> Void
    let lockWallet: () -> Void
    let sendWithoutPayjoin: (PayjoinSessionId) -> Void
    let size: CGSize

    var body: some View {
        ZStack(alignment: .bottom) {
            ScrollView {
                VStack(spacing: 24) {
                    SendFlowConfirmationSummary(
                        manager: manager,
                        details: details,
                        fiatAmount: fiatAmount,
                        showingMenu: $showingMenu
                    )

                    SendFlowAccountSection(manager: manager)
                        .padding(.top)

                    Divider()

                    SendFlowDetailsView(manager: manager, details: details, prices: prices)
                }
                .padding(.horizontal)
                .padding(.bottom, footerHeight)
            }
            .scrollIndicators(.hidden)
            .frame(width: size.width, height: size.height)
            .background(Color.coveBg)

            SendFlowConfirmationFooter(
                sendState: $sendState,
                lockingTask: $lockingTask,
                walletType: walletType,
                payjoinWaiting: payjoinWaiting,
                confirmSend: confirmSend,
                lockWallet: lockWallet,
                sendWithoutPayjoin: sendWithoutPayjoin
            )
            .offset(y: -sendConfirmationActionBottomPadding)
            .padding(.bottom, sendConfirmationActionBottomPadding)
            .background(Color.coveBg)
            .onGeometryChange(for: CGFloat.self) { proxy in
                proxy.size.height
            } action: { height in
                if footerHeight != height {
                    footerHeight = height
                }
            }
        }
    }
}

private struct SendFlowConfirmationSummary: View {
    @Bindable var manager: WalletManager

    let details: ConfirmDetails
    let fiatAmount: String
    @Binding var showingMenu: Bool

    var body: some View {
        VStack(spacing: 24) {
            SendFlowConfirmationTitle()
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
    var body: some View {
        VStack(spacing: 8) {
            HStack {
                Text("You're sending")
                    .font(.headline)
                    .fontWeight(.bold)

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

    let walletType: WalletType
    let payjoinWaiting: PayjoinNegotiation?
    let confirmSend: () -> Void
    let lockWallet: () -> Void
    let sendWithoutPayjoin: (PayjoinSessionId) -> Void

    var body: some View {
        VStack(spacing: 16) {
            if let payjoinWaiting {
                PayjoinWaitingCard(
                    negotiation: payjoinWaiting,
                    sendWithoutPayjoin: sendWithoutPayjoin
                )
                .id(payjoinWaiting.sessionId)
                .padding(.horizontal)
            }

            SwipeToSendView(
                sendState: $sendState,
                onConfirm: confirmSend
            )
            .padding(.horizontal)
        }
        .padding(.top, sendConfirmationActionTopPadding)
        .frame(maxWidth: .infinity)
        .fixedSize(horizontal: false, vertical: true)
        .background(Color.coveBg)
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
                                    input: .unsigned(mode: .standard)
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
            input: .unsigned(mode: .standard)
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
