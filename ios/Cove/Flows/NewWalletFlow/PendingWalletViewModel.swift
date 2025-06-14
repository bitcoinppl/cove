//
//  PendingWalletViewModel.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

@Observable final class PendingWalletManager: PendingWalletManagerReconciler {
    private let logger = Log(id: "PendingWalletManager")
    var rust: RustPendingWalletManager
    var numberOfWords: NumberOfBip39Words
    var bip39Words: [String]

    public init(numberOfWords: NumberOfBip39Words) {
        let rust = RustPendingWalletManager(numberOfWords: numberOfWords)
        self.rust = rust

        self.numberOfWords = numberOfWords
        bip39Words = rust.bip39Words()
        self.rust.listenForUpdates(reconciler: self)
    }

    func reconcile(message: PendingWalletManagerReconcileMessage) {
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

    public func dispatch(action: PendingWalletManagerAction) {
        rust.dispatch(action: action)
    }
}
