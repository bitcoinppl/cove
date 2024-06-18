//
//  WalletViewModel.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

@Observable class WalletViewModel: WalletViewModelReconciler {
    var rust: RustWalletViewModel
    var words: NumberOfBip39Words

    public init(words: NumberOfBip39Words) {
        self.rust = RustWalletViewModel(words: words)

        self.words = words
        self.rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                print("[swift] WalletViewModel Reconile: \(message)")

                switch message {
                case .words(let numberOfBip39Words):
                    self.words = numberOfBip39Words
                }
            }
        }
    }

    public func dispatch(action: WalletViewModelAction) {
        self.rust.dispatch(action: action)
    }
}
