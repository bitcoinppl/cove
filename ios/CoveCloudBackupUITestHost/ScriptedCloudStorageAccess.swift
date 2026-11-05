import CoveCore
import Foundation

final class ScriptedCloudStorageAccess: CloudStorageAccess, @unchecked Sendable {
    private let scenario: CloudBackupUITestScenario
    private let fixture: CloudBackupFixture
    private let lock = NSLock()
    private var uploadedNamespace: String?
    private var uploadedMaster: Data?
    private var uploadedWallets: [String: Data] = [:]
    /// Wallet record ids the app deleted, in call order; fixture wallets in it leave every listing
    private var deletedWalletRecordIds: [String] = []
    private var detailSnapshotAttempts = 0
    private var firstDetailCompletionDeadline: Date?

    init(scenario: CloudBackupUITestScenario) {
        self.scenario = scenario
        fixture = CloudBackupFixture.load()
    }

    func uploadMasterKeyBackup(
        namespace: String,
        location _: RemoteBackupLocation,
        data: Data,
        policy _: CloudAccessPolicy
    ) async throws {
        try validate(namespace: namespace)
        lock.withLock {
            uploadedNamespace = namespace
            uploadedMaster = data
        }
    }

    func uploadWalletBackup(
        namespace: String,
        recordId: String,
        location _: RemoteBackupLocation,
        data: Data,
        policy _: CloudAccessPolicy
    ) async throws {
        try validate(namespace: namespace)
        lock.withLock { uploadedWallets[recordId] = data }
    }

    func downloadMasterKeyBackup(
        namespace: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        try validate(namespace: namespace)

        return lock.withLock { uploadedMaster } ?? fixture.masterData
    }

    func downloadWalletBackup(
        namespace: String,
        recordId: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        try validate(namespace: namespace)

        if let uploaded = lock.withLock({ uploadedWallets[recordId] }) {
            return uploaded
        }
        if isDeletedFixtureWallet(recordId) {
            throw CloudStorageError.NotFound("cloud backup was deleted")
        }
        if scenario == .privateKeyWallet, recordId == fixture.xprvWallet.recordId {
            return fixture.xprvWallet.data
        }
        guard let wallet = fixture.walletsByRecordId[recordId] else {
            throw CloudStorageError.NotFound("cloud backup is not available")
        }

        return wallet.data
    }

    func deleteWalletBackup(
        namespace: String,
        recordId: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws {
        try validate(namespace: namespace)
        let deleted = lock.withLock {
            uploadedWallets[recordId] = nil
            deletedWalletRecordIds.append(recordId)
            return deletedWalletRecordIds
        }

        Self.writeDeleteLog(deleted)
    }

    func deleteNamespace(namespace: String, policy _: CloudAccessPolicy) async throws {
        try validate(namespace: namespace)
        lock.withLock {
            uploadedNamespace = nil
            uploadedMaster = nil
            uploadedWallets.removeAll()
        }
    }

    func listNamespaces(policy _: CloudAccessPolicy) async throws -> [String] {
        if scenario == .nativePasskeySmoke {
            return lock.withLock { uploadedNamespace.map { [$0] } ?? [] }
        }

        return [fixture.namespace]
    }

    func listWalletFiles(namespace: String, policy _: CloudAccessPolicy) async throws -> [String] {
        try validate(namespace: namespace)

        if scenario == .nativePasskeySmoke {
            return lock.withLock { uploadedWallets.keys.sorted() }
        }

        if scenario == .privateKeyWallet {
            return [fixture.xprvWallet.filename]
        }

        let detailAttempt = lock.withLock { () -> (attempt: Int, delay: TimeInterval)? in
            guard detailSnapshotAttempts > 0 else { return nil }

            let delay = firstDetailCompletionDeadline?.timeIntervalSinceNow ?? 0
            return (detailSnapshotAttempts, max(delay, 0))
        }
        // restore one wallet first so the detail snapshot can visibly retain one known row
        guard let detailAttempt else { return visibleFixtureFilenames(in: [fixture.wallets[0]]) }

        if detailAttempt.delay > 0 {
            try await Task.sleep(for: .seconds(detailAttempt.delay))
        }

        // rust performs one automatic connectivity retry before presenting Check Again
        if scenario == .timeoutThenRetry, detailAttempt.attempt <= 2 {
            throw CloudStorageError.NotAvailable("iCloud metadata query timed out")
        }

        return visibleFixtureFilenames(in: fixture.wallets)
    }

    func listWalletFilesSnapshot(
        namespace: String,
        policy _: CloudAccessPolicy
    ) async throws -> CloudStorageInventorySnapshot {
        try validate(namespace: namespace)

        if scenario == .nativePasskeySmoke {
            return CloudStorageInventorySnapshot(
                names: lock.withLock { uploadedWallets.keys.sorted() },
                isComplete: true
            )
        }

        if scenario == .privateKeyWallet {
            return CloudStorageInventorySnapshot(names: [fixture.xprvWallet.filename], isComplete: true)
        }

        lock.withLock {
            detailSnapshotAttempts += 1
            if detailSnapshotAttempts == 1 {
                firstDetailCompletionDeadline = Date().addingTimeInterval(6)
            }
        }
        return CloudStorageInventorySnapshot(
            names: visibleFixtureFilenames(in: [fixture.wallets[0]]),
            isComplete: false
        )
    }

    func isBackupUploaded(
        namespace: String,
        recordId: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> CloudBackupUploadStatus {
        try validate(namespace: namespace)

        if scenario == .nativePasskeySmoke {
            let exists = lock.withLock {
                if recordId == "cspp-master-key-v1" {
                    uploadedMaster != nil
                } else {
                    uploadedWallets[recordId] != nil
                }
            }

            return exists ? .uploaded : .notFound
        }

        return isDeletedFixtureWallet(recordId) ? .notFound : .uploaded
    }

    func overallSyncHealth(policy _: CloudAccessPolicy) async -> CloudSyncHealth {
        if scenario == .nativePasskeySmoke {
            return lock.withLock { uploadedMaster == nil ? .noFiles : .allUploaded }
        }

        return .allUploaded
    }

    private func isDeletedFixtureWallet(_ recordId: String) -> Bool {
        lock.withLock { deletedWalletRecordIds.contains(recordId) }
    }

    private func visibleFixtureFilenames(in wallets: [CloudBackupFixture.Wallet]) -> [String] {
        let deleted = lock.withLock { Set(deletedWalletRecordIds) }

        return wallets.filter { !deleted.contains($0.recordId) }.map(\.filename)
    }

    /// Writes every deleted record id as a JSON array to the file `COVE_CLOUD_BACKUP_UI_DELETE_LOG`
    /// names, so a UI test can check exactly which backups the app deleted
    private static func writeDeleteLog(_ recordIds: [String]) {
        guard let path = ProcessInfo.processInfo.environment["COVE_CLOUD_BACKUP_UI_DELETE_LOG"] else {
            return
        }

        // atomic, since the test polls the file while the app writes it; a failed write leaves the
        // delete itself alone and the test fails on the missing record id instead
        do {
            try JSONEncoder().encode(recordIds).write(to: URL(fileURLWithPath: path), options: .atomic)
        } catch {
            print("failed to write the scripted cloud delete log: \(error)")
        }
    }

    private func validate(namespace: String) throws {
        if scenario == .nativePasskeySmoke,
           namespace.count == 32,
           namespace.allSatisfy({ $0.isHexDigit && !$0.isUppercase })
        {
            return
        }

        guard namespace == fixture.namespace else {
            throw CloudStorageError.NotFound("cloud backup is not available")
        }
    }
}

private struct CloudBackupFixture: Decodable {
    struct Wallet: Decodable {
        let base64: String
        let filename: String
        let name: String
        let recordId: String
        let walletId: String

        var data: Data {
            guard let data = Data(base64Encoded: base64) else {
                fatalError("invalid generated cloud backup wallet fixture")
            }

            return data
        }

        enum CodingKeys: String, CodingKey {
            case base64
            case filename
            case name
            case recordId = "record_id"
            case walletId = "wallet_id"
        }
    }

    let masterBase64: String
    let namespace: String
    let wallets: [Wallet]
    /// Hot wallet holding only an extended private key, from the Rust fixture generator
    let xprvWallet: Wallet

    var masterData: Data {
        guard let data = Data(base64Encoded: masterBase64) else {
            fatalError("invalid generated cloud backup master fixture")
        }

        return data
    }

    var walletsByRecordId: [String: Wallet] {
        Dictionary(uniqueKeysWithValues: wallets.map { ($0.recordId, $0) })
    }

    static func load() -> Self {
        guard let url = Bundle.main.url(
            forResource: "CloudBackupFixture",
            withExtension: "json"
        ) else {
            fatalError("generated cloud backup fixture is missing")
        }

        do {
            return try JSONDecoder().decode(Self.self, from: Data(contentsOf: url))
        } catch {
            fatalError("generated cloud backup fixture is invalid: \(error)")
        }
    }

    enum CodingKeys: String, CodingKey {
        case masterBase64 = "master_base64"
        case namespace
        case wallets
        case xprvWallet = "xprv_wallet"
    }
}
