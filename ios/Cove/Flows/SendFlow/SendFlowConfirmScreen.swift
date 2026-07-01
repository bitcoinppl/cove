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
private let sendConfirmationActionReservedHeight: CGFloat = 70
    + sendConfirmationActionTopPadding
    + sendConfirmationActionBottomPadding

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
        // signed psbt has not been finalized yet
        if case let .signedPsbt(psbt) = input, finalizedTransaction == nil {
            FullPageLoadingView()
                .task {
                    do {
                        finalizedTransaction = try await manager.rust.finalizePsbt(psbt: psbt)
                    } catch let error as WalletManagerError {
                        app.alertState = .init(.general(title: "Unable to finalize transaction", message: error.description))
                    } catch {
                        app.alertState = .init(.general(title: "Unknown error", message: error.localizedDescription))
                    }
                }
        } else {
            VStack(spacing: 0) {
                // MARK: HEADER

                SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

                // MARK: CONTENT

                GeometryReader { geometry in
                    VStack(spacing: 0) {
                        ScrollView {
                            VStack(spacing: 24) {
                                // set amount
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
                                .padding(.top, 10)

                                // the amount in sats or btc
                                VStack(spacing: 8) {
                                    HStack(alignment: .bottom) {
                                        Spacer()

                                        Text(manager.amountFmt(details.sendingAmount()))
                                            .frame(minWidth: screenWidth / 2)
                                            .font(.system(size: 48, weight: .bold))
                                            .minimumScaleFactor(0.01)
                                            .lineLimit(1)
                                            .multilineTextAlignment(.center)

                                        Button(action: { showingMenu.toggle() }) {
                                            HStack(spacing: 0) {
                                                Text(metadata.selectedUnit == .sat ? "sats" : "btc")

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
                                            VStack(alignment: .center, spacing: 0) {
                                                Button("sats") {
                                                    manager.dispatch(action: .updateUnit(.sat))
                                                    showingMenu = false
                                                }
                                                .padding(12)
                                                .buttonStyle(.plain)

                                                Divider()

                                                Button("btc") {
                                                    manager.dispatch(action: .updateUnit(.btc))
                                                    showingMenu = false
                                                }
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
                                    .frame(alignment: .center)

                                    Text(fiatAmount)
                                        .font(.title3)
                                        .foregroundColor(.secondary)
                                }
                                .padding(.top, 8)

                                SendFlowAccountSection(manager: manager)
                                    .padding(.top)

                                Divider()

                                SendFlowDetailsView(manager: manager, details: details, prices: prices)
                            }
                        }
                        .scrollIndicators(.hidden)
                        .padding(.horizontal)
                        .frame(
                            width: geometry.size.width,
                            height: max(0, geometry.size.height - sendConfirmationActionReservedHeight)
                        )
                        .background(Color.coveBg)

                        SwipeToSendView(sendState: $sendState) {
                            sendState = .sending
                            Task {
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
                                        // for payjoin, stay in .sending — PayjoinTxBroadcast reconcile fires sendState = .sent
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
                        .frame(maxWidth: .infinity)
                        .offset(y: -sendConfirmationActionBottomPadding)
                        .padding(.horizontal)
                        .padding(.top, sendConfirmationActionTopPadding)
                        .padding(.bottom, sendConfirmationActionBottomPadding)
                        .background(Color.coveBg)
                        .onAppear {
                            lockingTask = Task {
                                try? await Task.sleep(for: .milliseconds(50))
                                if Task.isCancelled { return }

                                if metadata.walletType == .hot { auth.lock() }
                            }
                        }
                    }
                    .frame(width: geometry.size.width, height: geometry.size.height)
                    .background(Color.coveBg)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.coveBg)
            .onDisappear {
                lockingTask?.cancel()
                guard let lockedAt = auth.lockedAt else { return }
                let sinceLocked = Date.now.timeIntervalSince(lockedAt)
                if sinceLocked < 5 { auth.lockState = .unlocked }
            }
            .onChange(of: manager.payjoinTxBroadcast) { _, uuid in
                // UUID changes each time so this fires reliably across multiple sends
                guard uuid != nil, case .sending = sendState else { return }
                sendState = .sent
                presenter.confirmationAlertState = .init(.sent(id))
                auth.unlock()
            }
            .onChange(of: manager.sendFlowErrorAlert) { _, alert in
                // payjoin broadcast failure arrives via reconcile (not the catch block),
                // so we must handle it here to unblock the UI from .sending
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
            .presentingAlert(
                presenter.confirmationAlertStateBinding,
                context: confirmationAlertContext,
                defaultTitle: "Error Broadcasting!"
            )
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
