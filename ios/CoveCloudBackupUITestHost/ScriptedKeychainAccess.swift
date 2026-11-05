import CoveCore
import Foundation

/// In-memory keychain for the scripted host
///
/// With a `storeURL` every change is also written to that file, and an existing file is loaded at
/// launch, so a test that kills and relaunches the app keeps the keys a real keychain would keep
final class ScriptedKeychainAccess: KeychainAccess, @unchecked Sendable {
    private let lock = NSLock()
    private let storeURL: URL?
    private var values: [String: String]

    init(storeURL: URL? = nil) {
        self.storeURL = storeURL
        values = storeURL
            .flatMap { try? Data(contentsOf: $0) }
            .flatMap { try? JSONDecoder().decode([String: String].self, from: $0) }
            ?? [:]
    }

    func save(key: String, value: String) throws {
        try lock.withLock {
            values[key] = value
            try persistLocked()
        }
    }

    func get(key: String) throws -> String? {
        lock.withLock { values[key] }
    }

    func delete(key: String) -> Bool {
        lock.withLock {
            let removed = values.removeValue(forKey: key) != nil
            try? persistLocked()

            return removed
        }
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

        try lock.withLock {
            values = values.filter { key, _ in
                !suffixes.contains { key.hasSuffix($0) }
            }
            try persistLocked()
        }
    }

    /// a real keychain write is durable once it returns, so the file is written before returning
    private func persistLocked() throws {
        guard let storeURL else { return }

        let data = try JSONEncoder().encode(values)
        try CloudBackupUITestControl.writeAtomically(data, to: storeURL)
    }
}
