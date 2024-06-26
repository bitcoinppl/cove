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
    var focusField: Int?

    public init(numberOfWords: NumberOfBip39Words) {
        let rust = RustWalletViewModel(numberOfWords: numberOfWords)
        self.rust = rust

        self.numberOfWords = numberOfWords
        bip39Words = rust.bip39Words()
        self.rust.listenForUpdates(reconciler: self)
    }

    func submitWordField(fieldNumber: UInt8) {
        focusField = Int(fieldNumber) + 1
    }

    func reconcile(message: WalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                print("[swift] WalletViewModel Reconcile: \(message)")

                switch message {
                case let .words(numberOfBip39Words):
                    self.numberOfWords = numberOfBip39Words
                    self.bip39Words = self.rust.bip39Words()
                }
            }
        }
    }

    public func dispatch(action: WalletViewModelAction) {
        rust.dispatch(action: action)
    }
}
