//
//  SendFlowConfirmScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowConfirmScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth

    let id: WalletId
    @State var manager: WalletManager
    let details: ConfirmDetails
    @State var signedTransaction: BitcoinTransaction?
    let signedPsbt: Psbt?

    let prices: PriceResponse? = nil

    // private
    @State private var isShowingAlert = false
    @State private var sendState: SendState = .idle
    @State private var isShowingErrorAlert = false

    // popover to change btc and sats
    @State private var showingMenu: Bool = false

    // locking task, cancel if its screen is leaving
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

    var body: some View {
        // signed psbt has not been finalized yet
        if let psbt = signedPsbt, signedTransaction == nil {
            FullPageLoadingView()
                .task {
                    do {
                        signedTransaction = try await manager.rust.finalizePsbt(psbt: psbt)
                    } catch let error as WalletManagerError {
                        app.alertState = .init(.general(title: "Unable to finalize transaction", message: error.describe))
                    } catch {
                        app.alertState = .init(.general(title: "Unknown error", message: error.localizedDescription))
                    }
                }
        } else {
            VStack(spacing: 0) {
                // MARK: HEADER

                SendFlowHeaderView(manager: manager, amount: manager.balance.spendable())

                // MARK: CONTENT

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
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.coveBg)

                SwipeToSendView(sendState: $sendState) {
                    sendState = .sending
                    Task {
                        do {
                            if let txn = signedTransaction {
                                _ = try await manager.rust.broadcastTransaction(
                                    signedTransaction: txn)
                            } else {
                                _ = try await manager.rust.signAndBroadcastTransaction(
                                    psbt: details.psbt())
                            }
                            sendState = .sent
                            isShowingAlert = true
                            auth.unlock()
                        } catch let error as WalletManagerError {
                            sendState = .error(error.describe)
                            isShowingErrorAlert = true
                        } catch {
                            sendState = .error(error.localizedDescription)
                            isShowingErrorAlert = true
                        }
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.horizontal)
                .padding(.bottom, 6)
                .padding(.top, 20)
                .background(Color.coveBg)
                .onAppear {
                    lockingTask = Task {
                        try? await Task.sleep(for: .milliseconds(50))
                        if Task.isCancelled { return }

                        if metadata.walletType == .hot { auth.lock() }
                    }
                }
            }
            .onDisappear {
                lockingTask?.cancel()
                guard let lockedAt = auth.lockedAt else { return }
                let sinceLocked = Date.now.timeIntervalSince(lockedAt)
                if sinceLocked < 5 { auth.lockState = .unlocked }
            }
            .alert(
                "Sent!",
                isPresented: $isShowingAlert,
                actions: {
                    Button("OK") {
                        app.loadAndReset(to: Route.selectedWallet(id))
                    }
                },
                message: {
                    Text("Transaction was successfully sent!")
                }
            )
            .alert(
                "Error Broadcasting!",
                isPresented: $isShowingErrorAlert,
                actions: {
                    Button("OK") {
                        sendState = .idle
                        isShowingErrorAlert = false
                    }
                },
                message: {
                    if case let .error(error) = sendState {
                        Text(error)
                    } else {
                        Text(
                            "Unknown error, unable to broadcast transaction, please try again!")
                    }
                }
            )
        }
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
                                    details: ConfirmDetails.previewNew(amount: 30333),
                                    signedTransaction: nil,
                                    signedPsbt: nil
                                )
                                .environment(AppManager.shared)
                                .environment(AuthManager.shared)
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
            details: ConfirmDetails.previewNew(amount: 30_333_312),
            signedTransaction: nil, signedPsbt: nil
        )
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
    }
}
