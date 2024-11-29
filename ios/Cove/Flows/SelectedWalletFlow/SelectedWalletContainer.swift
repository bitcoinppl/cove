//
//  SelectedWalletContainer.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SelectedWalletContainer: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    let id: WalletId
    @State private var model: WalletViewModel? = nil

    func loadModel() {
        if model != nil { return }

        do {
            Log.debug("Getting wallet \(id)")
            model = try app.getWalletViewModel(id: id)
        } catch {
            Log.error("Something went very wrong: \(error)")
            navigate(Route.listWallets)
        }
    }

    var body: some View {
        Group {
            if let model {
                SelectedWalletScreen(model: model)
                    .background(
                        model.loadState == .loading
                            ? LinearGradient(
                                colors: [
                                    .black.opacity(colorScheme == .dark ? 0.9 : 0),
                                    .black.opacity(colorScheme == .dark ? 0.9 : 0),
                                ], startPoint: .top, endPoint: .bottom)
                            : LinearGradient(
                                stops: [
                                    .init(
                                        color: .midnightBlue,
                                        location: 0.20),
                                    .init(
                                        color: colorScheme == .dark ? .black.opacity(0.9) : .white,
                                        location: 0.20),
                                ], startPoint: .top, endPoint: .bottom)
                    )
                    .background(Color.white)

            } else {
                Text("Loading...")
            }
        }
        .onAppear {
            loadModel()
        }
        .task {
            // small delay and then start scanning wallet
            if let model {
                do {
                    try? await Task.sleep(for: .milliseconds(400))
                    try await model.rust.startWalletScan()
                } catch {
                    Log.error("Wallet Scan Failed \(error.localizedDescription)")
                }
            }
        }
        .onChange(of: model?.loadState) { _, loadState in
            if case .loaded = loadState {
                if let model {
                    app.updateWalletVm(model)
                }
            }
        }
    }
}

#Preview {
    SelectedWalletContainer(id: WalletId())
        .environment(MainViewModel())
}
