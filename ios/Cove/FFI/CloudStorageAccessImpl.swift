import CoveCore
import Foundation

final class CloudStorageAccessImpl: CloudStorageAccess, @unchecked Sendable {
    private let helper = ICloudDriveHelper.shared
    private let queue = DispatchQueue(
        label: "cove.CloudStorageAccessImpl",
        qos: .userInitiated
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

    func uploadMasterKeyBackup(namespace: String, data: Data) async throws {
        try await run {
            let url = try self.helper.masterKeyFileURL(namespace: namespace)
            try self.helper.writeForUpload(data: data, to: url)
            try self.helper.waitForMetadataVisibility(url: url)
        }
    }

    func uploadWalletBackup(namespace: String, recordId: String, data: Data) async throws {
        try await run {
            let url = try self.helper.walletFileURL(namespace: namespace, recordId: recordId)
            try self.helper.writeForUpload(data: data, to: url)
            try self.helper.waitForMetadataVisibility(url: url)
        }
    }

    // MARK: - Download

    func downloadMasterKeyBackup(namespace: String) async throws -> Data {
        try await run {
            let url = try self.helper.masterKeyFileReadURL(namespace: namespace)
            return try self.helper.downloadFile(url: url, recordId: csppMasterKeyRecordId())
        }
    }

    func downloadWalletBackup(namespace: String, recordId: String) async throws -> Data {
        try await run {
            let url = try self.helper.walletFileReadURL(namespace: namespace, recordId: recordId)
            return try self.helper.downloadFile(url: url, recordId: recordId)
        }
    }

    func deleteWalletBackup(namespace: String, recordId: String) async throws {
        try await run {
            let url = try self.helper.walletFileReadURL(namespace: namespace, recordId: recordId)
            if FileManager.default.fileExists(atPath: url.path) {
                try self.helper.coordinatedDelete(at: url, missingItemID: recordId)
                return
            }

            let resolvedURL = try self.helper.metadataItemIfPresent(
                named: url.lastPathComponent,
                parentDirectoryURL: url.deletingLastPathComponent()
            )?.url
            guard let resolvedURL else { throw CloudStorageError.NotFound(recordId) }

            try self.helper.coordinatedDelete(at: resolvedURL, missingItemID: recordId)
        }
    }

    // MARK: - Discovery

    func listNamespaces() async throws -> [String] {
        try await run {
            let namespacesRoot = try self.helper.namespacesRootURL()
            return try self.helper.listSubdirectories(parentPath: namespacesRoot.path)
        }
    }

    func listWalletFiles(namespace: String) async throws -> [String] {
        try await run {
            let namespacesRoot = try self.helper.namespacesRootReadURL()
            let namespaces = try self.helper.listSubdirectories(parentPath: namespacesRoot.path)
            guard namespaces.contains(namespace) else { throw CloudStorageError.NotFound(namespace) }

            let nsDir = try self.helper.namespaceDirectoryReadURL(namespace: namespace)
            return try self.helper.listFiles(namespacePath: nsDir.path, prefix: csppWalletFilePrefix())
        }
    }

    func isBackupUploaded(namespace: String, recordId: String) async throws -> Bool {
        try await run {
            try self.helper.isBackupUploaded(namespace: namespace, recordId: recordId)
        }
    }

    func overallSyncHealth() async -> CloudSyncHealth {
        await run {
            self.helper.overallSyncHealth()
        }
    }
}
