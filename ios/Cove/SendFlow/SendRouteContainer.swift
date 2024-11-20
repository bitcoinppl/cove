//
//  SendRouteContainer.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//
import Foundation
import SwiftUI

public struct SendRouteContainer: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    // passed in
    let sendRoute: SendRoute

    // private
    @State private var model: WalletViewModel? = nil
    @State private var presenter: SendFlowSetAmountPresenter? = nil

    func initOnAppear() {
        let id = sendRoute.id()
        if model != nil { return }

        do {
            Log.debug("Getting wallet for SendRoute \(id)")
            let model = try app.getWalletViewModel(id: id)

            self.model = model
            presenter = SendFlowSetAmountPresenter(app: app, model: model)
        } catch {
            Log.error("Something went very wrong: \(error)")
            navigate(Route.listWallets)
        }
    }

    public var body: some View {
        if let model, let presenter {
            Group {
                switch sendRoute {
                case let .setAmount(id: id, address: address, amount: amount):
                    SendFlowSetAmountScreen(id: id, model: model, address: address?.string() ?? "", amount: amount)
                case let .confirm(id: id, details: details):
                    SendFlowConfirmScreen(id: id, model: model, details: details)
                }
            }
            .environment(model)
            .environment(presenter)
            .onAppear {
                // if zero balance, show alert and send back
                if model.balance.confirmed.asSats() == 0 {
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
