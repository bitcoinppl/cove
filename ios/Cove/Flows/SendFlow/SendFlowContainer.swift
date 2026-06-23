//
//  SendRouteContainer.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//
import Foundation
import SwiftUI

public struct SendFlowContainer: View {
    @Environment(AppManager.self) private var app

    /// passed in
    let sendRoute: SendRoute

    public var body: some View {
        WalletManagerHost(walletId: sendRoute.id(), loading: {
            ProgressView()
                .tint(.primary)
        }, onError: { error in
            Log.error("Something went very wrong: \(error)")
            app.trySelectLatestOrNewWallet()
        }) { manager in
            SendFlowLoadedView(sendRoute: sendRoute, manager: manager)
        }
    }
}

private struct SendFlowLoadedView: View {
    @Environment(AppManager.self) private var app

    let sendRoute: SendRoute
    let manager: WalletManager

    @State private var initializedSendFlowManagerId: ObjectIdentifier?

    private var sendFlowManager: SendFlowManager? {
        app.cachedSendFlowManager(id: manager.id)
    }

    private var initializedSendFlowManager: SendFlowManager? {
        guard let sendFlowManager else { return nil }

        guard initializedSendFlowManagerId == ObjectIdentifier(sendFlowManager) else {
            return nil
        }

        return sendFlowManager
    }

    var body: some View {
        Group {
            if let sendFlowManager = initializedSendFlowManager {
                loadedContent(sendFlowManager: sendFlowManager)
            } else {
                ProgressView()
                    .tint(.primary)
            }
        }
        .task(id: ObjectIdentifier(manager)) {
            await ensureSendFlowManagerInitialized()
        }
    }

    @ViewBuilder
    private func loadedContent(sendFlowManager: SendFlowManager) -> some View {
        let presenter = sendFlowManager.presenter

        Group {
            sendRouteToScreen(sendRoute: sendRoute, manager: manager)
        }
        .environment(presenter)
        .environment(sendFlowManager)
        .onAppear {
            if manager.ledgerState.initialScanIncomplete {
                app.showInitialScanIncompleteAlert()
                app.popRoute()
                return
            }

            if manager.balance.spendable().asSats() == 0 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                    withAnimation(.easeInOut(duration: 0.4)) {
                        presenter.focusField = .none
                    }
                }

                presenter.alertState = .init(.error(.NoBalance))
                return
            }
        }
        .alert(
            alertTitle,
            isPresented: showingAlert,
            presenting: manager.sendFlowErrorAlert,
            actions: { myAlert($0).actions },
            message: { myAlert($0).message }
        )
        .onDisappear {
            presenter.setDisappearing()
        }
    }

    @ViewBuilder
    private func sendRouteToScreen(
        sendRoute: SendRoute,
        manager: WalletManager
    ) -> some View {
        switch sendRoute {
        case let .setAmount(id: id, address: _, amount: amount):
            SendFlowSetAmountScreen(id: id, amount: amount)
        case let .coinControlSetAmount(id: id, utxos: utxos):
            SendFlowCoinControlSetAmountScreen(id: id, utxos: utxos)
        case let .confirm(confirm):
            SendFlowConfirmScreen(
                id: confirm.id, manager: manager,
                details: confirm.details,
                input: confirm.input,
                payjoinEndpoint: confirm.payjoinEndpoint
            )
        case let .hardwareExport(id: id, details: details):
            SendFlowHardwareScreen(id: id, manager: manager, details: details)
        }
    }

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { manager.sendFlowErrorAlert != nil },
            set: { if !$0 { manager.sendFlowErrorAlert = .none } }
        )
    }

    private var alertTitle: String {
        guard let alert = manager.sendFlowErrorAlert else { return "Error!" }
        return myAlert(alert).title
    }

    private func myAlert(_ alert: TaggedItem<SendFlowErrorAlert>) -> AnyAlertBuilder {
        let error =
            switch alert.item {
            case let .confirmDetails(error): error
            case let .signAndBroadcast(error): error
            }

        return
            AlertBuilder(
                title: "Error!",
                message: error,
                actions: {
                    Button("OK", action: { manager.sendFlowErrorAlert = .none })
                }
            ).eraseToAny()
    }

    private func applyRouteArguments(to sendFlowManager: SendFlowManager) {
        switch sendRoute {
        case let .setAmount(id: _, address: address, amount: amount):
            if let address { sendFlowManager.setAddress(address) }
            if let amount { sendFlowManager.setAmount(amount) }
        default:
            ()
        }
    }

    @MainActor
    private func ensureSendFlowManagerInitialized() async {
        if initializedSendFlowManager != nil { return }

        let presenter = SendFlowPresenter(app: app, manager: manager)
        let sendFlowManager: SendFlowManager
        do {
            sendFlowManager = try app.ensureSendFlowManager(manager, presenter: presenter)
        } catch {
            handleSendFlowManagerError(error)
            return
        }
        let sendFlowManagerId = ObjectIdentifier(sendFlowManager)

        initializedSendFlowManagerId = nil
        applyRouteArguments(to: sendFlowManager)

        // rust handles alert + popRoute on failure
        if await sendFlowManager.rust.waitForInit() {
            guard !Task.isCancelled else { return }

            initializedSendFlowManagerId = sendFlowManagerId
        }
    }

    private func handleSendFlowManagerError(_ error: Error) {
        switch error {
        case WalletManagerError.InitialScanIncomplete:
            app.showInitialScanIncompleteAlert()
            app.popRoute()
        case let WalletManagerError.DatabaseCorruption(walletId, errorMessage):
            Log.error("Wallet database corrupted for \(walletId): \(errorMessage)")
            app.alertState = TaggedItem(
                .walletDatabaseCorrupted(walletId: walletId, error: errorMessage)
            )
            app.popRoute()
        case WalletManagerError.WalletDoesNotExist:
            Log.error("Wallet does not exist for send route \(sendRoute)")
            app.alertState = .init(.general(
                title: "Wallet Not Found",
                message: "This wallet is no longer available."
            ))
            app.trySelectLatestOrNewWallet()
        case let walletError as WalletManagerError:
            Log.error("Unable to open wallet for send flow: \(walletError)")
            app.alertState = .init(.general(
                title: "Unable to Open Wallet",
                message: "The wallet could not be opened for sending. Please try again from the wallet screen."
            ))
            app.popRoute()
        default:
            Log.error("Unable to open wallet for send flow: \(error)")
            app.alertState = .init(.general(
                title: "Unable to Open Wallet",
                message: "The wallet could not be opened for sending. Please try again from the wallet screen."
            ))
            app.popRoute()
        }
    }
}
