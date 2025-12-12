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
    @Environment(\.navigate) private var navigate

    // passed in
    let sendRoute: SendRoute

    // private
    @State private var manager: WalletManager? = nil
    @State private var sendFlowManager: SendFlowManager? = nil
    @State private var initCompleted: Bool = false

    func initOnAppear() {
        let id = sendRoute.id()
        if manager != nil { return }

        do {
            Log.debug("Getting wallet for SendRoute \(id)")
            let manager = try app.getWalletManager(id: id)

            let presenter = SendFlowPresenter(app: app, manager: manager)
            let sendFlowManager = app.getSendFlowManager(manager, presenter: presenter)

            switch sendRoute {
            case let .setAmount(id: _, address: address, amount: amount):
                if let address { sendFlowManager.setAddress(address) }
                if let amount { sendFlowManager.setAmount(amount) }
            default:
                ()
            }

            waitForInit()
            self.manager = manager
            self.sendFlowManager = sendFlowManager
        } catch {
            Log.error("Something went very wrong: \(error)")
            app.rust.selectLatestOrNewWallet()
        }
    }

    func waitForInit() {
        Task {
            let success = await sendFlowManager?.rust.waitForInit() ?? false
            await MainActor.run {
                // rust handles alert + popRoute on failure
                if success { initCompleted = true }
            }
        }
    }

    @ViewBuilder
    func sendRouteToScreen(
        sendRoute: SendRoute, manager: WalletManager, sendFlowManager _: SendFlowManager
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
                signedTransaction: confirm.signedTransaction,
                signedPsbt: confirm.signedPsbt
            )
        case let .hardwareExport(id: id, details: details):
            SendFlowHardwareScreen(id: id, manager: manager, details: details)
        }
    }

    public var body: some View {
        if let manager, let sendFlowManager, initCompleted {
            let presenter = sendFlowManager.presenter

            Group {
                sendRouteToScreen(
                    sendRoute: sendRoute, manager: manager, sendFlowManager: sendFlowManager
                )
            }
            .environment(manager)
            .environment(presenter)
            .environment(sendFlowManager)
            .onAppear {
                // if zero balance, show alert and send back
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
                actions: { MyAlert($0).actions },
                message: { MyAlert($0).message }
            )
            .onDisappear {
                presenter.setDisappearing()
            }

        } else {
            ProgressView()
                .tint(.primary)
                .onAppear(perform: initOnAppear)
        }
    }

    // MARK: Alerts

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { manager?.sendFlowErrorAlert != nil },
            set: { if !$0 { manager?.sendFlowErrorAlert = .none } }
        )
    }

    private var alertTitle: String {
        guard let alert = manager?.sendFlowErrorAlert else { return "Error!" }
        return MyAlert(alert).title
    }

    private func MyAlert(_ alert: TaggedItem<SendFlowErrorAlert>) -> AnyAlertBuilder {
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
                    Button("OK", action: { manager?.sendFlowErrorAlert = .none })
                }
            ).eraseToAny()
    }
}
