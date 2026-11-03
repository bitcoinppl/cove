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

enum KeychainItemReadResult: Sendable {
    case value(String)
    case itemNotFound
    case failure(OSStatus)
}

/// Separates a missing item from a read failure such as a locked device
///
/// Only `errSecItemNotFound` proves absence. Every other failure must reach Rust as an error so
/// it never replaces or deletes secrets that are still stored
func classifyKeychainItemRead(status: OSStatus, result: CFTypeRef?) -> KeychainItemReadResult {
    switch status {
    case errSecSuccess:
        guard let data = result as? Data, let value = String(data: data, encoding: .utf8) else {
            return .failure(errSecDecode)
        }

        return .value(value)

    case errSecItemNotFound:
        return .itemNotFound

    default:
        return .failure(status)
    }
}

/// Reads one item with the same query `KeychainSwift.get` uses, keeping the `OSStatus`
private func readGenericPassword(
    key: String,
    accessGroup: String?,
    synchronizable: Bool
) -> KeychainItemReadResult {
    var query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrAccount as String: key,
        kSecMatchLimit as String: kSecMatchLimitOne,
        kSecReturnData as String: true,
    ]
    if let accessGroup {
        query[kSecAttrAccessGroup as String] = accessGroup
    }
    if synchronizable {
        query[kSecAttrSynchronizable as String] = kSecAttrSynchronizableAny
    }

    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)

    return classifyKeychainItemRead(status: status, result: result)
}

/// Replaces an existing item in place, adding it only when absent
///
/// `KeychainSwift.set` deletes the old item before adding the new one, so a failed add loses the
/// value it was replacing. An update keeps the old value whenever the write fails
private func writeGenericPassword(key: String, value: String, accessGroup: String?) -> OSStatus {
    var query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrAccount as String: key,
    ]
    if let accessGroup {
        query[kSecAttrAccessGroup as String] = accessGroup
    }

    let attributes: [String: Any] = [
        kSecValueData as String: Data(value.utf8),
        kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
    ]

    let updateStatus = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
    guard updateStatus == errSecItemNotFound else { return updateStatus }

    return SecItemAdd(query.merging(attributes) { _, new in new } as CFDictionary, nil)
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

    /// Serializes direct reads and writes with deletes, which still go through `KeychainSwift`
    private let lock = NSLock()
    private let itemReader: @Sendable (String) -> KeychainItemReadResult
    private let accountEnumerator: @Sendable () -> KeychainAccountEnumerationResult

    init(
        keychain: KeychainSwift = KeychainSwift(),
        itemReader: (@Sendable (String) -> KeychainItemReadResult)? = nil,
        accountEnumerator: @escaping @Sendable () -> KeychainAccountEnumerationResult =
            { enumerateGenericPasswordAccounts() }
    ) {
        keychain.synchronizable = false

        // `KeychainSwift.get` drops the OSStatus, so reads query the keychain directly
        let accessGroup = keychain.accessGroup
        self.keychain = keychain
        self.itemReader = itemReader ?? { key in
            readGenericPassword(key: key, accessGroup: accessGroup, synchronizable: false)
        }
        self.accountEnumerator = accountEnumerator
    }

    func save(key: String, value: String) throws {
        let accessGroup = keychain.accessGroup
        let status = lock.withLock {
            writeGenericPassword(key: key, value: value, accessGroup: accessGroup)
        }

        if status != errSecSuccess {
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "unknown error"
            Log.error("Unable to save keychain item, status=\(status): \(message)")
            throw KeychainError.Save
        }
    }

    func get(key: String) throws -> String? {
        switch lock.withLock({ itemReader(key) }) {
        case let .value(value):
            value
        case .itemNotFound:
            nil
        case let .failure(status):
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "unknown error"
            Log.error("Unable to read keychain item, status=\(status): \(message)")
            throw KeychainError.Read("status=\(status): \(message)")
        }
    }

    func delete(key: String) -> Bool {
        lock.withLock { keychain.delete(key) }
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
        for key in walletKeys where !delete(key: key) {
            failed = true
        }

        if failed {
            throw KeychainError.Delete
        }
    }
}
