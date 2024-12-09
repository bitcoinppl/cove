//
//  SendRouteContainer.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//
import Foundation
import SwiftUI

public struct SendRouteContainer: View {
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate

    // passed in
    let sendRoute: SendRoute

    // private
    @State private var manager: WalletManager? = nil
    @State private var presenter: SendFlowSetAmountPresenter? = nil

    func initOnAppear() {
        let id = sendRoute.id()
        if manager != nil { return }

        do {
            Log.debug("Getting wallet for SendRoute \(id)")
            let manager = try app.getWalletManager(id: id)

            self.manager = manager
            presenter = SendFlowSetAmountPresenter(app: app, manager: manager)
        } catch {
            Log.error("Something went very wrong: \(error)")
            navigate(Route.listWallets)
        }
    }

    public var body: some View {
        if let manager, let presenter {
            Group {
                switch sendRoute {
                case let .setAmount(id: id, address: address, amount: amount):
                    SendFlowSetAmountScreen(
                        id: id, manager: manager, address: address?.string() ?? "", amount: amount)
                case let .confirm(id: id, details: details):
                    SendFlowConfirmScreen(id: id, manager: manager, details: details)
                case let .hardwareExport(id: id, details: details):
                    SendFlowHardwareScreen(id: id, manager: manager, details: details)
                }
            }
            .environment(manager)
            .environment(presenter)
            .onAppear {
                presenter.disappearing = false

                // if zero balance, show alert and send back
                if manager.balance.confirmed.asSats() == 0 {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                        withAnimation(.easeInOut(duration: 0.4)) {
                            presenter.focusField = .none
                        }
                    }

                    presenter.setAlertState(.noBalance)
                    return
                }
            }
            .onDisappear {
                presenter.disappearing = true
            }

        } else {
            ProgressView()
                .onAppear(perform: initOnAppear)
        }
    }
}
