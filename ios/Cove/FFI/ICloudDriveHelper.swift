import CoveCore
import Foundation

final class ICloudDriveHelper: Sendable {
    static let shared = ICloudDriveHelper()

    private let containerIdentifier = "iCloud.com.covebitcoinwallet"
    let defaultTimeout: TimeInterval = 60
    let metadataSettleInterval: TimeInterval = 0.5

    private let fileCoordinationClient = FileCoordinationClient()
    private let stateMachine = UploadDownloadStateMachine()

    final class ObserverBox {
        private var observers: [NSObjectProtocol] = []

        func add(_ observer: NSObjectProtocol) {
            observers.append(observer)
        }

        func removeAll() {
            for observer in observers {
                NotificationCenter.default.removeObserver(observer)
            }
            observers.removeAll()
        }
    }

    enum MetadataLookupError: LocalizedError {
        case startFailed(String)
        case timedOut(String)
        case missingURL(String)

        var errorDescription: String? {
            switch self {
            case let .startFailed(message),
                 let .timedOut(message),
                 let .missingURL(message):
                message
            }
        }
    }

    // MARK: - Path mapping

    func containerURL() throws -> URL {
        guard
            let url = FileManager.default.url(
                forUbiquityContainerIdentifier: containerIdentifier
            )
        else {
            throw CloudStorageError.NotAvailable("iCloud Drive is not available")
        }
        return url
    }

    func dataDirectoryURL() throws -> URL {
        let url = try paths().dataDirectoryURL()
        try fileCoordinationClient.createDirectory(at: url)
        return url
    }

    func dataDirectoryReadURL() throws -> URL {
        try paths().dataDirectoryURL()
    }

    func namespacesRootURL() throws -> URL {
        _ = try dataDirectoryURL()
        let url = try paths().namespacesRootURL()
        try fileCoordinationClient.createDirectory(at: url)
        return url
    }

    func namespacesRootReadURL() throws -> URL {
        try paths().namespacesRootURL()
    }

    func validateNamespace(_ namespace: String) throws -> String {
        try ICloudPaths.validateNamespace(namespace)
    }

    func namespaceDirectoryURL(namespace: String) throws -> URL {
        let namespace = try validateNamespace(namespace)
        let url = try namespacesRootURL()
            .appendingPathComponent(namespace, isDirectory: true)
        try fileCoordinationClient.createDirectory(at: url)
        return url
    }

    func namespaceDirectoryReadURL(namespace: String) throws -> URL {
        try paths().namespaceDirectoryURL(namespace: namespace)
    }

    func walletsDirectoryReadURL(namespace: String) throws -> URL {
        try paths().walletsDirectoryURL(namespace: namespace)
    }

    func walletLocation(filename: String) -> String {
        ICloudPaths.walletLocation(filename: filename)
    }

    func backupFileURL(namespace: String, location: RemoteBackupLocation) throws -> URL {
        _ = try namespaceDirectoryURL(namespace: namespace)
        let url = try paths().backupFileURL(namespace: namespace, location: location)
        try fileCoordinationClient.createDirectory(at: url.deletingLastPathComponent())
        return url
    }

    func backupFileReadURL(namespace: String, location: RemoteBackupLocation) throws -> URL {
        try paths().backupFileURL(namespace: namespace, location: location)
    }

    func existingBackupFileReadURL(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation]
    ) throws -> URL {
        var lastError: Error?
        for location in locations {
            do {
                let url = try backupFileReadURL(namespace: namespace, location: location)
                if FileManager.default.fileExists(atPath: url.path) {
                    return url
                }

                let item = try metadataItemIfPresent(
                    named: url.lastPathComponent,
                    parentDirectoryURL: url.deletingLastPathComponent()
                )
                if let item { return item.url }
            } catch {
                lastError = error
            }
        }

        if let lastError { throw lastError }
        throw CloudStorageError.NotFound(recordId)
    }

    private func paths() throws -> ICloudPaths {
        try ICloudPaths(containerURL: containerURL())
    }

    // MARK: - File coordination

    func coordinatedWrite(data: Data, to url: URL) throws {
        try fileCoordinationClient.write(data: data, to: url)
    }

    func writeForUpload(data: Data, to url: URL) throws {
        try fileCoordinationClient.writeForUpload(data: data, to: url)
    }

    func coordinatedDelete(at url: URL, missingItemID: String) throws {
        try fileCoordinationClient.delete(at: url, missingItemID: missingItemID)
    }

    func coordinatedRead(from url: URL) throws -> Data {
        try fileCoordinationClient.read(from: url)
    }

    /// Downloads a file from iCloud via coordinated read
    ///
    /// Tries startDownloadingUbiquitousItem as a hint, then uses NSFileCoordinator
    /// which forces the download through a different (more reliable) path
    func downloadFile(url: URL, recordId: String) throws -> Data {
        try stateMachine.downloadFile(
            url: url,
            recordId: recordId,
            fileExists: { FileManager.default.fileExists(atPath: $0.path) },
            downloadState: { self.downloadState(for: $0) },
            waitForMetadataItem: { name, parentDirectoryURL, deadline in
                try self.waitForMetadataItem(
                    named: name,
                    parentDirectoryURL: parentDirectoryURL,
                    deadline: deadline
                )
            },
            triggerDownload: { url, recordId, filename in
                try self.triggerDownload(url: url, recordId: recordId, filename: filename)
            },
            coordinatedRead: { try self.coordinatedRead(from: $0) }
        )
    }

    // MARK: - Upload verification

    func waitForMetadataVisibility(url: URL) throws {
        try stateMachine.waitForMetadataVisibility(
            url: url,
            waitForMetadataItem: { name, parentDirectoryURL, deadline in
                try self.waitForMetadataItem(
                    named: name,
                    parentDirectoryURL: parentDirectoryURL,
                    deadline: deadline
                )
            }
        )
    }

    func waitForUpload(url: URL) throws {
        try stateMachine.waitForUpload(
            url: url,
            waitForMetadataItem: { name, parentDirectoryURL, deadline in
                try self.waitForMetadataItem(
                    named: name,
                    parentDirectoryURL: parentDirectoryURL,
                    deadline: deadline
                )
            },
            uploadState: { self.uploadState(for: $0) },
            uploadDiagnostics: { self.fileCoordinationClient.uploadDiagnostics(for: $0) },
            logMetadataItems: { parentDirectoryURL, reason, focusName in
                self.logMetadataItems(
                    under: parentDirectoryURL,
                    reason: reason,
                    focusName: focusName
                )
            }
        )
    }

    // MARK: - Download

    func ensureDownloaded(url: URL, recordId: String) throws {
        _ = try downloadFile(url: url, recordId: recordId)
    }

    private func triggerDownload(url: URL, recordId: String, filename _: String) throws {
        do {
            try FileManager.default.startDownloadingUbiquitousItem(at: url)
        } catch {
            if FileCoordinationClient.isFileReadNoSuchFileError(error) {
                throw CloudStorageError.NotFound(recordId)
            }
            Log.warn("downloadFile: startDownloading failed: \(error.localizedDescription)")
        }
    }

    private func uploadState(for url: URL) -> UploadDownloadStateMachine.UploadState {
        // clear cached resource values to prevent stale reads
        var freshURL = url
        freshURL.removeAllCachedResourceValues()

        guard
            let values = try? freshURL.resourceValues(forKeys: [
                .isUbiquitousItemKey,
                .ubiquitousItemIsUploadingKey,
                .ubiquitousItemIsUploadedKey,
                .ubiquitousItemUploadingErrorKey,
            ])
        else {
            return .unknown
        }

        guard values.isUbiquitousItem == true else { return .notUbiquitous }
        if values.ubiquitousItemIsUploaded == true { return .uploaded }
        if let error = values.ubiquitousItemUploadingError { return .failed(error) }

        return .uploading
    }

    private func downloadState(for url: URL) -> UploadDownloadStateMachine.DownloadState {
        var freshURL = url
        freshURL.removeAllCachedResourceValues()

        guard
            let values = try? freshURL.resourceValues(forKeys: [
                .isUbiquitousItemKey,
                .ubiquitousItemIsDownloadingKey,
                .ubiquitousItemDownloadingStatusKey,
                .ubiquitousItemDownloadingErrorKey,
            ])
        else {
            return .unknown
        }

        guard values.isUbiquitousItem == true else { return .notUbiquitous }
        if values.ubiquitousItemDownloadingStatus == .current { return .current }
        if let error = values.ubiquitousItemDownloadingError { return .failed(error) }
        if values.ubiquitousItemIsDownloading == true { return .downloading }

        return .notDownloaded
    }

    /// Lists immediate subdirectory names within a parent path
    ///
    /// Tries FileManager first (fast, sees .icloud stubs and dataless files),
    /// falls back to metadata query only if FileManager finds nothing
    func listSubdirectories(parentPath: String) throws -> [String] {
        let parentURL = URL(fileURLWithPath: parentPath, isDirectory: true)

        if let names = try? fileCoordinationClient.listSubdirectoriesViaFileManager(
            parentURL: parentURL
        ), !names.isEmpty {
            return names.sorted()
        }

        Log.info("listSubdirectories: FileManager found nothing, falling back to metadata query")
        return try metadataSubdirectoryNames(parentDirectoryURL: parentURL)
    }

    /// Lists filenames matching a prefix within a namespace directory
    ///
    /// Tries FileManager first (fast, sees .icloud stubs and dataless files),
    /// falls back to metadata query only if FileManager finds nothing
    func listFiles(namespacePath: String, prefix: String) throws -> [String] {
        let dirURL = URL(fileURLWithPath: namespacePath, isDirectory: true)

        if let names = try? fileCoordinationClient.listFilesViaFileManager(
            dirURL: dirURL,
            prefix: prefix
        ), !names.isEmpty {
            return names.sorted()
        }

        Log.info("listFiles: FileManager found nothing, falling back to metadata query prefix=\(prefix)")
        return try metadataFileNames(parentDirectoryURL: dirURL, prefix: prefix)
    }

    // MARK: - Upload status for UI

    enum UploadStatus {
        case uploaded
        case uploading
        case failed(String)
        case unknown
    }

    func uploadStatus(for url: URL) -> UploadStatus {
        guard FileManager.default.fileExists(atPath: url.path) else { return .unknown }

        switch uploadState(for: url) {
        case .uploaded: return .uploaded
        case let .failed(error): return .failed(error.localizedDescription)
        case .uploading, .notUbiquitous, .unknown: return .uploading
        }
    }

    func isBackupUploaded(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation]
    ) throws -> Bool {
        let urls = try locations.map { location in
            try backupFileReadURL(namespace: namespace, location: location)
        }

        for url in urls {
            let resolvedURL =
                resolvedMetadataItemIfPresent(
                    named: url.lastPathComponent,
                    parentDirectoryURL: url.deletingLastPathComponent()
                )?.url ?? url

            let state = uploadState(for: resolvedURL)
            let usedMetadata = resolvedURL != url
            Log.info(
                "isBackupUploaded: recordId=\(recordId.prefix(12))… path=\(url.path) state=\(state) usedMetadata=\(usedMetadata)"
            )

            if case .uploaded = state { return true }
        }

        return false
    }

    func deleteExistingBackupFile(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation]
    ) throws {
        var deletedAny = false
        var lastError: Error?

        let urls = try locations.map { location in
            try backupFileReadURL(namespace: namespace, location: location)
        }

        for url in urls {
            do {
                if FileManager.default.fileExists(atPath: url.path) {
                    try coordinatedDelete(at: url, missingItemID: recordId)
                    deletedAny = true
                    continue
                }

                let resolvedURL = try metadataItemIfPresent(
                    named: url.lastPathComponent,
                    parentDirectoryURL: url.deletingLastPathComponent()
                )?.url
                guard let resolvedURL else { continue }

                try coordinatedDelete(at: resolvedURL, missingItemID: recordId)
                deletedAny = true
            } catch CloudStorageError.NotFound {
                continue
            } catch {
                lastError = error
            }
        }

        if let lastError { throw lastError }
        if !deletedAny { throw CloudStorageError.NotFound(recordId) }
    }

    private func allBackupFiles(in namespaceDirectory: URL) -> [URL] {
        guard
            let enumerator = FileManager.default.enumerator(
                at: namespaceDirectory,
                includingPropertiesForKeys: nil
            )
        else {
            return []
        }

        return enumerator.compactMap { item -> URL? in
            guard let url = item as? URL else { return nil }
            guard url.pathExtension == "json" else { return nil }
            return url
        }
    }

    private func hasUploadedState(for file: URL) -> (hasFile: Bool, allUploaded: Bool, failed: String?) {
        let status = uploadStatus(for: file)
        switch status {
        case .uploaded:
            return (true, true, nil)
        case .uploading, .unknown:
            return (true, false, nil)
        case let .failed(message):
            return (true, false, message)
        }
    }

    static func isValidNamespaceDirectory(_ url: URL) -> Bool {
        ICloudPaths.isValidNamespaceDirectory(url)
    }

    /// Checks sync health of all files in namespace directories
    func overallSyncHealth() -> CloudSyncHealth {
        guard let namespacesRoot = try? namespacesRootURL() else { return .unavailable }

        guard
            let namespaceDirs = try? FileManager.default.contentsOfDirectory(
                at: namespacesRoot, includingPropertiesForKeys: nil,
                options: .skipsHiddenFiles
            )
        else {
            return .unavailable
        }

        var hasFiles = false
        var allUploaded = true
        var anyFailed = false
        var failureMessage: String?

        for nsDir in namespaceDirs where Self.isValidNamespaceDirectory(nsDir) {
            for file in allBackupFiles(in: nsDir) {
                let state = hasUploadedState(for: file)
                hasFiles = hasFiles || state.hasFile
                allUploaded = allUploaded && state.allUploaded
                if let failed = state.failed {
                    anyFailed = true
                    failureMessage = failed
                }
            }
        }

        if !hasFiles { return .noFiles }
        if anyFailed { return .failed(failureMessage ?? "upload error") }
        if allUploaded { return .allUploaded }
        return .uploading
    }
}
