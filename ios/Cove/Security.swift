//
//  Security.swift
//  Cove
//
//  Created by Praveen Perera on 6/19/24.
//

import Foundation
import KeychainSwift

class KeychainAccessor: KeychainAccess {
    let keychain: KeychainSwift

    init() {
        let keychain = KeychainSwift()
        keychain.synchronizable = false

        self.keychain = keychain
    }

    func save(key: String, value: String) throws {
        if !keychain.set(value, forKey: key) {
            throw KeychainError.Save
        }
    }

    func get(key: String) -> String? {
        keychain.get(key)
    }

    func delete(key: String) -> Bool {
        keychain.delete(key)
    }
}
