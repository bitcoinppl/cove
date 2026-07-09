import CoveCore
import Foundation

final class CancellableDispatchOperation<Value: Sendable>: @unchecked Sendable {
    private typealias Continuation = CheckedContinuation<Value, Error>

    private let lock = NSLock()
    private var continuation: Continuation?
    private var pendingResult: Result<Value, Error>?
    private var isResolved = false

    static func run(
        on queue: DispatchQueue,
        operation: @escaping @Sendable () throws -> Value
    ) async throws -> Value {
        let state = CancellableDispatchOperation()

        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                state.install(continuation)
                queue.async {
                    state.resolve(Result { try operation() })
                }
            }
        } onCancel: {
            state.resolve(.failure(CancellationError()))
        }
    }

    private func install(_ continuation: Continuation) {
        let pendingResult: Result<Value, Error>? = lock.withLock {
            guard isResolved else {
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
            guard !isResolved else { return nil }

            isResolved = true
            guard let continuation = self.continuation else {
                pendingResult = result
                return nil
            }

            self.continuation = nil
            return continuation
        }

        continuation?.resume(with: result)
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
            guard remaining > metadataTimeoutCleanupAllowance else { return [] }

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
            guard remainingBeforeDelay > 0 else { return [] }

            let delay = min(retryDelays[inspectionIndex], remainingBeforeDelay)
            try await sleep(delay)
        }

        return []
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
            try self.helper.waitForMetadataVisibility(url: url)
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
            try self.helper.waitForMetadataVisibility(url: url)
        }
    }

    // MARK: - Download

    func downloadMasterKeyBackup(
        namespace: String,
        locations: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        try await run {
            let url = try self.helper.existingBackupFileReadURL(
                namespace: namespace,
                recordId: csppMasterKeyRecordId(),
                locations: locations
            )
            return try self.helper.downloadFile(url: url, recordId: csppMasterKeyRecordId())
        }
    }

    func downloadWalletBackup(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        try await run {
            let url = try self.helper.existingBackupFileReadURL(
                namespace: namespace,
                recordId: recordId,
                locations: locations
            )
            return try self.helper.downloadFile(url: url, recordId: recordId)
        }
    }

    func deleteWalletBackup(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws {
        try await run {
            try self.helper.deleteExistingBackupFile(
                namespace: namespace,
                recordId: recordId,
                locations: locations
            )
        }
    }

    func deleteNamespace(namespace: String, policy _: CloudAccessPolicy) async throws {
        try await run {
            let url = try self.helper.namespaceDirectoryReadURL(namespace: namespace)
            if FileManager.default.fileExists(atPath: url.path) {
                try self.helper.coordinatedDelete(at: url, missingItemID: namespace)
                return
            }

            let resolvedURL = try self.helper.metadataItemIfPresent(
                named: url.lastPathComponent,
                parentDirectoryURL: url.deletingLastPathComponent()
            )?.url
            guard let resolvedURL else { throw CloudStorageError.NotFound(namespace) }

            try self.helper.coordinatedDelete(at: resolvedURL, missingItemID: namespace)
        }
    }

    // MARK: - Discovery

    func listNamespaces(policy: CloudAccessPolicy) async throws -> [String] {
        switch policy {
        case .consentAllowed:
            return try await run {
                let namespacesRoot = try self.helper.namespacesRootURL()
                return try self.helper.listSubdirectories(parentPath: namespacesRoot.path)
            }
        case .silent:
            let namespacesRoot = try await runCancellable {
                try self.helper.namespacesRootReadURL()
            }

            return try await SilentNamespaceRecoveryProbe.run { metadataTimeout in
                try await self.runCancellable {
                    try self.helper.listSubdirectories(
                        parentPath: namespacesRoot.path,
                        metadataTimeout: metadataTimeout
                    )
                }
            }
        }
    }

    func listWalletFiles(namespace: String, policy _: CloudAccessPolicy) async throws -> [String] {
        try await run {
            let namespace = try self.helper.validateNamespace(namespace)
            let namespacesRoot = try self.helper.namespacesRootReadURL()
            let namespaces = try self.helper.listSubdirectories(parentPath: namespacesRoot.path)
            guard namespaces.contains(namespace) else { throw CloudStorageError.NotFound(namespace) }

            let nsDir = try self.helper.namespaceDirectoryReadURL(namespace: namespace)
            let legacyFiles = try self.helper.listFiles(
                namespacePath: nsDir.path,
                prefix: csppWalletFilePrefix()
            )

            let walletsDir = try self.helper.walletsDirectoryReadURL(namespace: namespace)
            let currentFiles = try self.helper.listFiles(
                namespacePath: walletsDir.path,
                prefix: csppWalletFilePrefix()
            )
            .map { self.helper.walletLocation(filename: $0) }

            return Array(Set(legacyFiles + currentFiles)).sorted()
        }
    }

    func isBackupUploaded(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Bool {
        try await run {
            try self.helper.isBackupUploaded(
                namespace: namespace,
                recordId: recordId,
                locations: locations
            )
        }
    }

    func overallSyncHealth(policy _: CloudAccessPolicy) async -> CloudSyncHealth {
        await run {
            self.helper.overallSyncHealth()
        }
    }
}
