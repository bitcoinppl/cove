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
    let payjoinEndpoint: String?

    let prices: PriceResponse? = nil

    // private
    @State private var finalizedTransaction: BitcoinTransaction? = nil
    @State private var sendState: SendState = .idle

    @State private var sendConfirmationFooterHeight: CGFloat = 0

    /// popover to change btc and sats
    @State private var showingMenu: Bool = false

    /// locking task, cancel if its screen is leaving
    @State private var lockingTask: Task<Void, Never>? = nil

    var fiatAmount: String {
        guard let prices = prices ?? app.prices else {
            app.dispatch(action: .updateFiatPrices)
            return "---"
        }

        let amount = details.sendingAmount()
        return manager.rust.convertAndDisplayFiat(amount: amount, prices: prices)
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
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
                confirmSend: confirmSend,
                lockWallet: auth.lock
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.coveBg)
            .overlay(alignment: .bottom) {
                if case let .payjoinWaiting(deadline) = sendState {
                    PayjoinWaitingBanner(deadlineSecs: deadline, onCancel: cancelPayjoin)
                        .padding(.bottom, sendConfirmationFooterHeight + 16)
                }
            }
            .onDisappear(perform: handleDisappear)
            .onChange(of: manager.payjoinDeadlineSecs, payjoinPollingStarted)
            .onChange(of: manager.payjoinTxBroadcast, payjoinBroadcastChanged)
            .onChange(of: manager.sendFlowErrorAlert, sendFlowErrorChanged)
            .presentingAlert(
                presenter.confirmationAlertStateBinding,
                context: confirmationAlertContext,
                defaultTitle: "Error Broadcasting!"
            )
        }
    }

    private func confirmSend() {
        sendState = .sending
        Task { await sendTransaction() }
    }

    private func handleDisappear() {
        lockingTask?.cancel()
        guard let lockedAt = auth.lockedAt else { return }

        let sinceLocked = Date.now.timeIntervalSince(lockedAt)
        if sinceLocked < 5 { auth.lockState = .unlocked }
    }

    private func payjoinPollingStarted(_: UInt64?, _ deadline: UInt64?) {
        guard let deadline, case .sending = sendState else { return }
        sendState = .payjoinWaiting(deadlineSecs: deadline)
    }

    private func payjoinBroadcastChanged(_: UUID?, _ uuid: UUID?) {
        guard uuid != nil else { return }
        switch sendState {
        case .sending, .payjoinWaiting: break
        default: return
        }

        sendState = .sent
        presenter.confirmationAlertState = .init(.sent(id))
        auth.unlock()
    }

    private func cancelPayjoin() {
        Task {
            try? await manager.rust.cancelPayjoin()
        }
    }

    private func sendFlowErrorChanged(
        _: TaggedItem<SendFlowErrorAlert>?,
        _ alert: TaggedItem<SendFlowErrorAlert>?
    ) {
        switch sendState {
        case .sending, .payjoinWaiting: break
        default: return
        }
        guard let alert else { return }

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
                _ = try await manager.rust.broadcastTransaction(
                    signedTransaction: txn
                )
            case .signedPsbt:
                guard let finalizedTransaction else {
                    throw SendConfirmationError.unfinalizedSignedPsbt
                }
                _ = try await manager.rust.broadcastTransaction(
                    signedTransaction: finalizedTransaction
                )
            case .unsigned:
                _ = try await manager.rust.initiatePayment(
                    psbt: details.psbt(),
                    payjoinEndpoint: payjoinEndpoint
                )
                // for payjoin, stay in .sending until PayjoinTxBroadcast reconciles
                if payjoinEndpoint == nil {
                    sendState = .sent
                    presenter.confirmationAlertState = .init(.sent(id))
                    auth.unlock()
                }
                return
            }
            sendState = .sent
            presenter.confirmationAlertState = .init(.sent(id))
            auth.unlock()
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
            finalizedTransaction = try await manager.rust.finalizePsbt(psbt: psbt)
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
    let confirmSend: () -> Void
    let lockWallet: () -> Void

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
    let prices: PriceResponse?
    let fiatAmount: String
    @Binding var showingMenu: Bool
    @Binding var sendState: SendState
    @Binding var footerHeight: CGFloat
    @Binding var lockingTask: Task<Void, Never>?
    let walletType: WalletType
    let confirmSend: () -> Void
    let lockWallet: () -> Void
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
                confirmSend: confirmSend,
                lockWallet: lockWallet
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
    let confirmSend: () -> Void
    let lockWallet: () -> Void

    var body: some View {
        VStack(spacing: 0) {
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
                        manager = WalletManager(preview: "preview_only", metadata)
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
            manager: WalletManager(preview: "preview_only"),
            details: confirmDetailsPreviewNew(),
            input: .unsigned,
            payjoinEndpoint: nil
        )
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
        .environment(
            SendFlowPresenter(
                app: AppManager.shared,
                manager: WalletManager(preview: "preview_only")
            )
        )
    }
}

private struct PayjoinWaitingBanner: View {
    let deadlineSecs: UInt64
    let onCancel: () -> Void

    @State private var remainingSecs: Int = 0

    var body: some View {
        VStack(spacing: 8) {
            ProgressView()
                .controlSize(.small)
            Text("Waiting for receiver")
                .font(.subheadline)
                .fontWeight(.semibold)
            Text("\(remainingSecs / 60):\(String(format: "%02d", remainingSecs % 60)) remaining")
                .font(.caption)
                .foregroundStyle(.secondary)
            Button("Cancel and send normally", action: onCancel)
                .font(.subheadline)
        }
        .padding()
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
        .padding(.horizontal, 16)
        .task {
            while !Task.isCancelled {
                let now = UInt64(Date().timeIntervalSince1970)
                remainingSecs = max(0, Int(deadlineSecs) - Int(now))
                if remainingSecs <= 0 { break }
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }
}
