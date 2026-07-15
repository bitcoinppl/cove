import CoveCore
import Foundation

final class CancellableDispatchOperation<Value: Sendable>: @unchecked Sendable {
    private typealias Continuation = CheckedContinuation<Value, Error>

    private enum State {
        case pending
        case running
        case resolved
    }

    private let lock = NSLock()
    private var continuation: Continuation?
    private var pendingResult: Result<Value, Error>?
    private var state = State.pending

    static func run(
        on queue: DispatchQueue,
        operation: @escaping @Sendable () throws -> Value
    ) async throws -> Value {
        let state = CancellableDispatchOperation()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                state.install(continuation)
                queue.async {
                    state.runIfPending(operation)
                }
            }
        } onCancel: {
            state.resolve(.failure(CancellationError()))
        }
    }

    private func install(_ continuation: Continuation) {
        let pendingResult: Result<Value, Error>? = lock.withLock {
            guard state == .resolved else {
                self.continuation = continuation
                return nil
            }

            let result = self.pendingResult
            self.pendingResult = nil
            return result
        }

        if let pendingResult {
            continuation.resume(with: pendingResult)
        }
    }

    private func resolve(_ result: Result<Value, Error>) {
        let continuation: Continuation? = lock.withLock {
            guard state != .resolved else { return nil }

            state = .resolved
            guard let continuation = self.continuation else {
                pendingResult = result
                return nil
            }

            self.continuation = nil
            return continuation
        }

        continuation?.resume(with: result)
    }

    private func runIfPending(_ operation: () throws -> Value) {
        let shouldRun = lock.withLock {
            guard state == .pending else { return false }

            state = .running
            return true
        }

        guard shouldRun else { return }

        resolve(Result { try operation() })
    }
}

enum SilentCloudRecoveryDeadline {
    typealias Watchdog = @Sendable () async throws -> Void

    private enum Outcome<Value: Sendable>: Sendable {
        case completed(Value)
        case timedOut
    }

    static func run<Value: Sendable>(
        watchdog: @escaping Watchdog = {
            try await Task.sleep(for: .seconds(SilentNamespaceRecoveryProbe.maximumDuration))
        },
        operation: @escaping @Sendable () async throws -> Value
    ) async throws -> Value {
        try await withThrowingTaskGroup(of: Outcome<Value>.self) { group in
            group.addTask {
                try await .completed(operation())
            }
            group.addTask {
                try await watchdog()
                return .timedOut
            }

            defer { group.cancelAll() }

            guard let outcome = try await group.next() else {
                throw CloudStorageError.NotAvailable("iCloud namespace lookup did not complete")
            }

            switch outcome {
            case let .completed(value):
                try Task.checkCancellation()
                return value
            case .timedOut:
                throw CloudStorageError.NotAvailable("iCloud namespace lookup timed out")
            }
        }
    }
}

enum SilentNamespaceRecoveryProbe {
    typealias Inspection = @Sendable (_ metadataTimeout: TimeInterval) async throws -> [String]
    typealias MonotonicNow = @Sendable () -> TimeInterval
    typealias Sleep = @Sendable (_ duration: TimeInterval) async throws -> Void

    static let maximumDuration: TimeInterval = 15
    static let maximumInspections = 4
    static let maximumMetadataTimeout: TimeInterval = 5
    static let metadataTimeoutCleanupAllowance: TimeInterval = 1
    static let retryDelays: [TimeInterval] = [1, 2, 4]

    static func run(
        now: @escaping MonotonicNow = { ProcessInfo.processInfo.systemUptime },
        sleep: @escaping Sleep = { duration in
            try await Task.sleep(for: .seconds(duration))
        },
        inspect: @escaping Inspection
    ) async throws -> [String] {
        let deadline = now() + maximumDuration

        for inspectionIndex in 0 ..< maximumInspections {
            try Task.checkCancellation()

            let remaining = deadline - now()
            guard remaining > metadataTimeoutCleanupAllowance else {
                throw CloudStorageError.NotAvailable("iCloud namespace lookup timed out")
            }

            let metadataTimeout = min(
                maximumMetadataTimeout,
                remaining - metadataTimeoutCleanupAllowance
            )
            let namespaces = try await inspect(metadataTimeout)

            try Task.checkCancellation()

            if !namespaces.isEmpty {
                return namespaces
            }

            guard inspectionIndex < retryDelays.count else { return [] }

            let remainingBeforeDelay = deadline - now()
            guard remainingBeforeDelay > 0 else {
                throw CloudStorageError.NotAvailable("iCloud namespace lookup timed out")
            }

            let delay = min(retryDelays[inspectionIndex], remainingBeforeDelay)
            try await sleep(delay)
        }

        return []
    }
}

enum ICloudEventuallyConsistentListing {
    static func merged(
        local: [String],
        metadata: Result<[String], Error>
    ) throws -> [String] {
        switch metadata {
        case let .success(metadata):
            return Array(Set(local + metadata)).sorted()
        case let .failure(error):
            throw error
        }
    }
}

final class CloudStorageAccessImpl: CloudStorageAccess, @unchecked Sendable {
    private let helper = ICloudDriveHelper.shared
    private let queue = DispatchQueue(
        label: "cove.CloudStorageAccessImpl",
        qos: .userInitiated,
        attributes: .concurrent
    )

    private func run<T: Sendable>(
        _ operation: @escaping @Sendable () throws -> T
    ) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                do {
                    try continuation.resume(returning: operation())
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    private func run<T: Sendable>(
        _ operation: @escaping @Sendable () -> T
    ) async -> T {
        await withCheckedContinuation { continuation in
            queue.async {
                continuation.resume(returning: operation())
            }
        }
    }

    private func runCancellable<T: Sendable>(
        _ operation: @escaping @Sendable () throws -> T
    ) async throws -> T {
        try await CancellableDispatchOperation.run(on: queue, operation: operation)
    }

    private func listSubdirectories(
        parentPath: String,
        metadataTimeout: TimeInterval? = nil
    ) async throws -> [String] {
        let localNames = try await runCancellable {
            self.helper.locallyVisibleSubdirectoryNames(parentPath: parentPath)
        }

        let metadataNames = try await helper.metadataSubdirectoryNames(
            parentPath: parentPath,
            timeout: metadataTimeout
        )
        let mergedNames = try ICloudEventuallyConsistentListing.merged(
            local: localNames,
            metadata: .success(metadataNames)
        )
        Log.info(
            "listSubdirectories: local_count=\(localNames.count) metadata_count=\(metadataNames.count) merged_count=\(mergedNames.count)"
        )
        return mergedNames
    }

    private func listFiles(namespacePath: String, prefix: String) async throws -> [String] {
        let localNames = try await runCancellable {
            self.helper.locallyVisibleFileNames(namespacePath: namespacePath, prefix: prefix)
        }

        let metadataNames = try await helper.metadataFileNames(
            namespacePath: namespacePath,
            prefix: prefix
        )
        let mergedNames = try ICloudEventuallyConsistentListing.merged(
            local: localNames,
            metadata: .success(metadataNames)
        )
        Log.info(
            "listFiles: prefix=\(prefix) local_count=\(localNames.count) metadata_count=\(metadataNames.count) merged_count=\(mergedNames.count)"
        )
        return mergedNames
    }

    // MARK: - Upload

    func uploadMasterKeyBackup(
        namespace: String,
        location: RemoteBackupLocation,
        data: Data,
        policy _: CloudAccessPolicy
    ) async throws {
        try await run {
            let url = try self.helper.backupFileURL(namespace: namespace, location: location)
            try self.helper.writeForUpload(data: data, to: url)
        }
    }

    func uploadWalletBackup(
        namespace: String,
        recordId _: String,
        location: RemoteBackupLocation,
        data: Data,
        policy _: CloudAccessPolicy
    ) async throws {
        try await run {
            let url = try self.helper.backupFileURL(namespace: namespace, location: location)
            try self.helper.writeForUpload(data: data, to: url)
        }
    }

    // MARK: - Download

    func downloadMasterKeyBackup(
        namespace: String,
        locations: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        let recordId = csppMasterKeyRecordId()
        let url = try await helper.existingBackupFileReadURL(
            namespace: namespace,
            recordId: recordId,
            locations: locations
        )
        return try await helper.downloadFile(url: url, recordId: recordId)
    }

    func downloadWalletBackup(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        let url = try await helper.existingBackupFileReadURL(
            namespace: namespace,
            recordId: recordId,
            locations: locations
        )
        return try await helper.downloadFile(url: url, recordId: recordId)
    }

    func deleteWalletBackup(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws {
        try await helper.deleteExistingBackupFile(
            namespace: namespace,
            recordId: recordId,
            locations: locations
        )
    }

    func deleteNamespace(namespace: String, policy _: CloudAccessPolicy) async throws {
        let url = try await run {
            try self.helper.namespaceDirectoryReadURL(namespace: namespace)
        }
        let isLocallyVisible = await run {
            FileManager.default.fileExists(atPath: url.path)
        }
        if isLocallyVisible {
            try await run {
                try self.helper.coordinatedDelete(at: url, missingItemID: namespace)
            }
            return
        }

        let resolvedURL = try await helper.metadataItemIfPresent(
            named: url.lastPathComponent,
            parentDirectoryURL: url.deletingLastPathComponent()
        )?.url
        guard let resolvedURL else { throw CloudStorageError.NotFound(namespace) }

        try await run {
            try self.helper.coordinatedDelete(at: resolvedURL, missingItemID: namespace)
        }
    }

    // MARK: - Discovery

    func listNamespaces(policy: CloudAccessPolicy) async throws -> [String] {
        switch policy {
        case .consentAllowed:
            let namespacesRoot = try await run {
                try self.helper.namespacesRootURL()
            }
            return try await listSubdirectories(parentPath: namespacesRoot.path)
        case .silent:
            return try await SilentCloudRecoveryDeadline.run {
                let namespacesRoot = try await self.runCancellable {
                    try self.helper.namespacesRootReadURL()
                }

                return try await SilentNamespaceRecoveryProbe.run { metadataTimeout in
                    try await self.listSubdirectories(
                        parentPath: namespacesRoot.path,
                        metadataTimeout: metadataTimeout
                    )
                }
            }
        }
    }

    func listWalletFiles(namespace: String, policy _: CloudAccessPolicy) async throws -> [String] {
        let directories = try await run {
            let namespace = try self.helper.validateNamespace(namespace)
            let namespacesRoot = try self.helper.namespacesRootReadURL()
            let nsDir = try self.helper.namespaceDirectoryReadURL(namespace: namespace)
            let walletsDir = try self.helper.walletsDirectoryReadURL(namespace: namespace)
            return (namespace, namespacesRoot, nsDir, walletsDir)
        }
        let namespaces = try await listSubdirectories(parentPath: directories.1.path)
        guard namespaces.contains(directories.0) else {
            throw CloudStorageError.NotFound(directories.0)
        }

        let legacyFiles = try await listFiles(
            namespacePath: directories.2.path,
            prefix: csppWalletFilePrefix()
        )
        let currentFiles = try await listFiles(
            namespacePath: directories.3.path,
            prefix: csppWalletFilePrefix()
        )
        .map { helper.walletLocation(filename: $0) }

        return Array(Set(legacyFiles + currentFiles)).sorted()
    }

    func listWalletFilesSnapshot(
        namespace: String,
        policy _: CloudAccessPolicy
    ) async throws -> CloudStorageInventorySnapshot {
        try await run {
            let namespace = try self.helper.validateNamespace(namespace)
            let namespaceDirectory = try self.helper.namespaceDirectoryReadURL(namespace: namespace)
            let legacyFiles =
                (try? self.helper.localFileSnapshot(
                    namespacePath: namespaceDirectory.path,
                    prefix: csppWalletFilePrefix()
                )) ?? []

            let walletsDirectory = try self.helper.walletsDirectoryReadURL(namespace: namespace)
            let currentFiles =
                ((try? self.helper.localFileSnapshot(
                    namespacePath: walletsDirectory.path,
                    prefix: csppWalletFilePrefix()
                )) ?? [])
                .map { self.helper.walletLocation(filename: $0) }

            return CloudStorageInventorySnapshot(
                names: Array(Set(legacyFiles + currentFiles)).sorted(),
                isComplete: false
            )
        }
    }

    func isBackupUploaded(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Bool {
        try await helper.isBackupUploaded(
            namespace: namespace,
            recordId: recordId,
            locations: locations
        )
    }

    func overallSyncHealth(policy _: CloudAccessPolicy) async -> CloudSyncHealth {
        await run {
            self.helper.overallSyncHealth()
        }
    }
}
