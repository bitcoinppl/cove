import CoveCore
import Foundation

final class CloudStorageAccessImpl: CloudStorageAccess, @unchecked Sendable {
    private let helper = ICloudDriveHelper.shared

    // MARK: - Upload

    func uploadMasterKeyBackup(namespace: String, data: Data) throws {
        let url = try helper.masterKeyFileURL(namespace: namespace)
        try helper.writeForUpload(data: data, to: url)
        try helper.waitForMetadataVisibility(url: url)
    }

    func uploadWalletBackup(namespace: String, recordId: String, data: Data) throws {
        let url = try helper.walletFileURL(namespace: namespace, recordId: recordId)
        try helper.writeForUpload(data: data, to: url)
        try helper.waitForMetadataVisibility(url: url)
    }

    // MARK: - Download

    func downloadMasterKeyBackup(namespace: String) throws -> Data {
        let url = try helper.masterKeyFileReadURL(namespace: namespace)
        return try helper.downloadFile(url: url, recordId: csppMasterKeyRecordId())
    }

    func downloadWalletBackup(namespace: String, recordId: String) throws -> Data {
        let url = try helper.walletFileReadURL(namespace: namespace, recordId: recordId)
        return try helper.downloadFile(url: url, recordId: recordId)
    }

    func deleteWalletBackup(namespace: String, recordId: String) throws {
        let url = try helper.walletFileReadURL(namespace: namespace, recordId: recordId)
        if FileManager.default.fileExists(atPath: url.path) {
            try helper.coordinatedDelete(at: url, missingItemID: recordId)
            return
        }

        let resolvedURL = try helper.metadataItemIfPresent(
            named: url.lastPathComponent,
            parentDirectoryURL: url.deletingLastPathComponent()
        )?.url
        guard let resolvedURL else { throw CloudStorageError.NotFound(recordId) }

        try helper.coordinatedDelete(at: resolvedURL, missingItemID: recordId)
    }

    // MARK: - Discovery

    func listNamespaces() throws -> [String] {
        let namespacesRoot = try helper.namespacesRootURL()
        return try helper.listSubdirectories(parentPath: namespacesRoot.path)
    }

    func listWalletFiles(namespace: String) throws -> [String] {
        let namespacesRoot = try helper.namespacesRootReadURL()
        let namespaces = try helper.listSubdirectories(parentPath: namespacesRoot.path)
        guard namespaces.contains(namespace) else { throw CloudStorageError.NotFound(namespace) }

        let nsDir = try helper.namespaceDirectoryReadURL(namespace: namespace)
        return try helper.listFiles(namespacePath: nsDir.path, prefix: csppWalletFilePrefix())
    }

    func isBackupUploaded(namespace: String, recordId: String) throws -> Bool {
        try helper.isBackupUploaded(namespace: namespace, recordId: recordId)
    }

    func overallSyncHealth() -> CloudSyncHealth {
        helper.overallSyncHealth()
    }
}
