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

    func initOnAppear() {
        let id = sendRoute.id()
        if model != nil { return }

        do {
            Log.debug("Getting wallet for SendRoute \(id)")
            model = try app.getWalletViewModel(id: id)
        } catch {
            Log.error("Something went very wrong: \(error)")
            navigate(Route.listWallets)
        }
    }

    public var body: some View {
        if let model = model {
            switch sendRoute {
            case let .setAmount(id: id, address: address, amount: amount):
                SendFlowSetAmountScreen(id: id, model: model, address: address?.string() ?? "", amount: amount)
            case let .confirm(id: id, details: details):
                SendFlowConfirmScreen(id: id, model: model, details: details)
            }
        } else {
            ProgressView()
                .onAppear(perform: initOnAppear)
        }
    }
}
