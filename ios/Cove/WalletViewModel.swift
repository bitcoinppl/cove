//
//  WalletViewModel.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

@Observable class WalletViewModel: WalletViewModelReconciler {
    var rust: RustWalletViewModel
    var numberOfWords: NumberOfBip39Words
    var bip39Words: [String]

    public init(numberOfWords: NumberOfBip39Words) {
        let rust = RustWalletViewModel(numberOfWords: numberOfWords)
        self.rust = rust

        self.numberOfWords = numberOfWords
        self.bip39Words = rust.bip39Words()
        self.rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                print("[swift] WalletViewModel Reconile: \(message)")

                switch message {
                case .words(let numberOfBip39Words):
                    self.numberOfWords = numberOfBip39Words
                    self.bip39Words = self.rust.bip39Words()
                }
            }
        }
    }

    public func dispatch(action: WalletViewModelAction) {
        print(Thread.current)
        self.rust.dispatch(action: action)
    }
}
