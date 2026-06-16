import CoveCore
import Foundation

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

    func listNamespaces(policy _: CloudAccessPolicy) async throws -> [String] {
        try await run {
            let namespacesRoot = try self.helper.namespacesRootURL()
            return try self.helper.listSubdirectories(parentPath: namespacesRoot.path)
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
