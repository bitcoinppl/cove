//
//  Security.swift
//  Cove
//
//  Created by Praveen Perera on 6/19/24.
//

import Foundation
import KeychainSwift
import Security

enum KeychainAccountEnumerationResult: Sendable {
    case success([String])
    case itemNotFound
    case failure(OSStatus)
}

func classifyKeychainAccountEnumeration(
    status: OSStatus,
    result: CFTypeRef?
) -> KeychainAccountEnumerationResult {
    switch status {
    case errSecSuccess:
        guard let items = result as? [[String: Any]] else {
            return .failure(errSecDecode)
        }

        let accounts = items.compactMap { $0[kSecAttrAccount as String] as? String }
        guard accounts.count == items.count else {
            return .failure(errSecDecode)
        }

        return .success(accounts)

    case errSecItemNotFound:
        return .itemNotFound

    default:
        return .failure(status)
    }
}

private func enumerateGenericPasswordAccounts() -> KeychainAccountEnumerationResult {
    let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecReturnAttributes as String: true,
        kSecMatchLimit as String: kSecMatchLimitAll,
    ]
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)

    return classifyKeychainAccountEnumeration(status: status, result: result)
}

class KeychainAccessor: KeychainAccess {
    let keychain: KeychainSwift
    private let accountEnumerator: @Sendable () -> KeychainAccountEnumerationResult

    init(
        keychain: KeychainSwift = KeychainSwift(),
        accountEnumerator: @escaping @Sendable () -> KeychainAccountEnumerationResult =
            { enumerateGenericPasswordAccounts() }
    ) {
        keychain.synchronizable = false

        self.keychain = keychain
        self.accountEnumerator = accountEnumerator
    }

    func save(key: String, value: String) throws {
        if !keychain.set(
            value,
            forKey: key,
            withAccess: .accessibleWhenUnlockedThisDeviceOnly
        ) {
            throw KeychainError.Save
        }
    }

    func get(key: String) -> String? {
        keychain.get(key)
    }

    func delete(key: String) -> Bool {
        keychain.delete(key)
    }

    func deleteAllWalletItems() throws {
        let suffixes = [
            "::wallet_mnemonic",
            "::wallet_mnemonic_encryption_key_and_nonce",
            "::wallet_xpub",
            "::wallet_public_descriptor",
            "::tap_signer_backup",
            "::wallet_tap_signer_encryption_key_and_nonce_key_name",
        ]
        let accounts: [String]
        switch accountEnumerator() {
        case let .success(enumeratedAccounts):
            accounts = enumeratedAccounts
        case .itemNotFound:
            accounts = []
        case let .failure(status):
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "unknown error"
            Log.error("Unable to enumerate wallet keychain items, status=\(status): \(message)")
            throw KeychainError.Delete
        }

        let walletKeys = accounts.filter { key in
            suffixes.contains { key.hasSuffix($0) }
        }
        var failed = false
        for key in walletKeys where !keychain.delete(key) {
            failed = true
        }

        if failed {
            throw KeychainError.Delete
        }
    }
}
