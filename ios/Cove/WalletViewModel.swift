//
//  WalletViewModel.swift
//  Cove
//
//  Created by Praveen Perera on 6/27/24.
//

import SwiftUI

@Observable class WalletViewModel: WalletViewModelReconciler {
    var rust: RustWalletViewModel

    public init(id: WalletId) {
        let rust = RustWalletViewModel(id: id)
        self.rust = rust

        rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                print("[swift] WalletViewModel Reconcile: \(message)")

                switch message {}
            }
        }
    }

    public func dispatch(action: WalletViewModelAction) {
        rust.dispatch(action: action)
    }
}
