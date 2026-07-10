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
}
