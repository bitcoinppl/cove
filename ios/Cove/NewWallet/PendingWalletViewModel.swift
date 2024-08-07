//
//  PendingWalletViewModel.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

@Observable class PendingWalletViewModel: PendingWalletViewModelReconciler {
    private let logger = Log(id: "PendingWalletViewModel")
    var rust: RustPendingWalletViewModel
    var numberOfWords: NumberOfBip39Words
    var bip39Words: [String]

    public init(numberOfWords: NumberOfBip39Words) {
        let rust = RustPendingWalletViewModel(numberOfWords: numberOfWords)
        self.rust = rust

        self.numberOfWords = numberOfWords
        bip39Words = rust.bip39Words()
        self.rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: PendingWalletViewModelReconcileMessage) {
        Task {
            await MainActor.run {
                logger.debug("Reconcile: \(message)")

                switch message {
                case let .words(numberOfBip39Words):
                    self.numberOfWords = numberOfBip39Words
                    self.bip39Words = self.rust.bip39Words()
                }
            }
        }
    }

    public func dispatch(action: PendingWalletViewModelAction) {
        rust.dispatch(action: action)
    }
}
