import CoveCore
import Foundation

final class ScriptedKeychainAccess: KeychainAccess, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String] = [:]

    func save(key: String, value: String) throws {
        lock.withLock { values[key] = value }
    }

    func get(key: String) -> String? {
        lock.withLock { values[key] }
    }

    func delete(key: String) -> Bool {
        lock.withLock { values.removeValue(forKey: key) != nil }
    }

    /// Mirrors `KeychainAccessor`: removes only wallet-scoped items and keeps app-level keys
    func deleteAllWalletItems() throws {
        let suffixes = [
            "::wallet_mnemonic",
            "::wallet_mnemonic_encryption_key_and_nonce",
            "::wallet_xpub",
            "::wallet_public_descriptor",
            "::tap_signer_backup",
            "::wallet_tap_signer_encryption_key_and_nonce_key_name",
        ]

        lock.withLock {
            values = values.filter { key, _ in
                !suffixes.contains { key.hasSuffix($0) }
            }
        }
    }
}
