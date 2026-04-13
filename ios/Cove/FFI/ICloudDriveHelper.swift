import CoveCore
import Foundation

final class ICloudDriveHelper: @unchecked Sendable {
    static let shared = ICloudDriveHelper()

    private let containerIdentifier = "iCloud.com.covebitcoinwallet"
    private let dataSubdirectory = "Data"
    private let namespacesSubdirectory = csppNamespacesSubdirectory()
    let defaultTimeout: TimeInterval = 60
    private let pollInterval: TimeInterval = 0.1
    let metadataSettleInterval: TimeInterval = 0.5
    private let progressLogInterval: TimeInterval = 1

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

    struct ResolvedMetadataItem {
        let url: URL
        let metadataPath: String?
    }

    private enum UploadState: CustomStringConvertible {
        case uploaded
        case uploading
        case failed(Error)
        case notUbiquitous
        case unknown

        var description: String {
            switch self {
            case .uploaded: "uploaded"
            case .uploading: "uploading"
            case let .failed(error): "failed: \(error.localizedDescription)"
            case .notUbiquitous: "not ubiquitous"
            case .unknown: "unknown"
            }
        }
    }

    private enum DownloadState: CustomStringConvertible {
        case current
        case downloading
        case failed(Error)
        case notUbiquitous
        case notDownloaded
        case unknown

        var description: String {
            switch self {
            case .current: "current"
            case .downloading: "downloading"
            case let .failed(error): "failed: \(error.localizedDescription)"
            case .notUbiquitous: "not ubiquitous"
            case .notDownloaded: "not downloaded"
            case .unknown: "unknown"
            }
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

    private static func isConnectivityError(_ error: Error) -> Bool {
        if let urlError = error as? URLError {
            return [
                .notConnectedToInternet,
                .networkConnectionLost,
                .timedOut,
                .cannotFindHost,
                .cannotConnectToHost,
                .dnsLookupFailed,
                .internationalRoamingOff,
                .dataNotAllowed,
            ].contains(urlError.code)
        }

        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain {
            return [
                NSURLErrorNotConnectedToInternet,
                NSURLErrorNetworkConnectionLost,
                NSURLErrorTimedOut,
                NSURLErrorCannotFindHost,
                NSURLErrorCannotConnectToHost,
                NSURLErrorDNSLookupFailed,
                NSURLErrorInternationalRoamingOff,
                NSURLErrorDataNotAllowed,
            ].contains(nsError.code)
        }

        return false
    }

    private static func uploadError(_ context: String, error: Error) -> CloudStorageError {
        if isConnectivityError(error) {
            return .Offline("\(context): \(error.localizedDescription)")
        }

        return .UploadFailed("\(context): \(error.localizedDescription)")
    }

    private static func downloadError(_ context: String, error: Error) -> CloudStorageError {
        if isConnectivityError(error) {
            return .Offline("\(context): \(error.localizedDescription)")
        }

        return .DownloadFailed("\(context): \(error.localizedDescription)")
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
        let url = try containerURL().appendingPathComponent(dataSubdirectory, isDirectory: true)
        try coordinatedCreateDirectory(at: url)
        return url
    }

    func dataDirectoryReadURL() throws -> URL {
        try containerURL().appendingPathComponent(dataSubdirectory, isDirectory: true)
    }

    /// Root directory for all namespaces: Data/cspp-namespaces/
    func namespacesRootURL() throws -> URL {
        let url = try dataDirectoryURL()
            .appendingPathComponent(namespacesSubdirectory, isDirectory: true)
        try coordinatedCreateDirectory(at: url)
        return url
    }

    func namespacesRootReadURL() throws -> URL {
        try dataDirectoryReadURL()
            .appendingPathComponent(namespacesSubdirectory, isDirectory: true)
    }

    /// Directory for a specific namespace: Data/cspp-namespaces/{namespace}/
    func namespaceDirectoryURL(namespace: String) throws -> URL {
        let url = try namespacesRootURL()
            .appendingPathComponent(namespace, isDirectory: true)
        try coordinatedCreateDirectory(at: url)
        return url
    }

    func namespaceDirectoryReadURL(namespace: String) throws -> URL {
        try namespacesRootReadURL()
            .appendingPathComponent(namespace, isDirectory: true)
    }

    func masterKeyFileURL(namespace: String) throws -> URL {
        let filename = csppMasterKeyFilename()
        return try namespaceDirectoryURL(namespace: namespace)
            .appendingPathComponent(filename)
    }

    func masterKeyFileReadURL(namespace: String) throws -> URL {
        let filename = csppMasterKeyFilename()
        return try namespaceDirectoryReadURL(namespace: namespace)
            .appendingPathComponent(filename)
    }

    func walletFileURL(namespace: String, recordId: String) throws -> URL {
        let filename = csppWalletFilenameFromRecordId(recordId: recordId)
        return try namespaceDirectoryURL(namespace: namespace)
            .appendingPathComponent(filename)
    }

    func walletFileReadURL(namespace: String, recordId: String) throws -> URL {
        let filename = csppWalletFilenameFromRecordId(recordId: recordId)
        return try namespaceDirectoryReadURL(namespace: namespace)
            .appendingPathComponent(filename)
    }

    func backupFileURL(namespace: String, recordId: String) throws -> URL {
        if recordId == csppMasterKeyRecordId() { return try masterKeyFileURL(namespace: namespace) }

        return try walletFileURL(namespace: namespace, recordId: recordId)
    }

    // MARK: - File coordination

    /// Coordinates iCloud-backed filesystem access because ubiquitous items may
    /// be placeholders, may resolve to a different concrete URL, and can fail
    /// if we touch them directly without asking the system to arbitrate first
    private func coordinatedCreateDirectory(at url: URL) throws {
        guard !FileManager.default.fileExists(atPath: url.path) else { return }

        var coordinatorError: NSError?
        var createError: Error?

        let coordinator = NSFileCoordinator()
        coordinator.coordinate(writingItemAt: url, options: [], error: &coordinatorError) {
            newURL in
            do {
                try FileManager.default.createDirectory(
                    at: newURL,
                    withIntermediateDirectories: true
                )
            } catch {
                createError = error
            }
        }

        if let error = coordinatorError ?? createError {
            throw Self.uploadError("create directory failed", error: error)
        }
    }

    func coordinatedWrite(data: Data, to url: URL) throws {
        var coordinatorError: NSError?
        var writeError: Error?

        let coordinator = NSFileCoordinator()
        coordinator.coordinate(
            writingItemAt: url, options: .forReplacing, error: &coordinatorError
        ) { newURL in
            do {
                try data.write(to: newURL, options: .atomic)
            } catch {
                writeError = error
            }
        }

        if let error = coordinatorError ?? writeError {
            throw Self.uploadError("write failed", error: error)
        }
    }

    func writeForUpload(data: Data, to url: URL) throws {
        guard !FileManager.default.fileExists(atPath: url.path) else {
            try coordinatedWrite(data: data, to: url)
            return
        }

        let tempURL = FileManager.default.temporaryDirectory.appendingPathComponent(
            "icloud-upload-\(UUID().uuidString)-\(url.lastPathComponent)"
        )

        do {
            try data.write(to: tempURL, options: .atomic)
        } catch {
            throw CloudStorageError.UploadFailed(
                "temporary write failed: \(error.localizedDescription)"
            )
        }

        defer {
            if FileManager.default.fileExists(atPath: tempURL.path) {
                try? FileManager.default.removeItem(at: tempURL)
            }
        }

        Log.info(
            "writeForUpload: staging first upload via setUbiquitous for \(url.lastPathComponent)"
        )

        var coordinatorError: NSError?
        var moveError: Error?

        let coordinator = NSFileCoordinator()
        coordinator.coordinate(writingItemAt: url, options: [], error: &coordinatorError) {
            destinationURL in
            do {
                try FileManager.default.setUbiquitous(
                    true,
                    itemAt: tempURL,
                    destinationURL: destinationURL
                )
            } catch {
                moveError = error
            }
        }

        if let error = coordinatorError ?? moveError {
            throw Self.uploadError("setUbiquitous failed", error: error)
        }
    }

    func coordinatedDelete(at url: URL, missingItemID: String) throws {
        var coordinatorError: NSError?
        var deleteError: Error?

        let coordinator = NSFileCoordinator()
        coordinator.coordinate(
            writingItemAt: url, options: .forDeleting, error: &coordinatorError
        ) { newURL in
            do {
                try FileManager.default.removeItem(at: newURL)
            } catch {
                deleteError = error
            }
        }

        if let error = coordinatorError ?? deleteError {
            if Self.isNoSuchFileError(error) { throw CloudStorageError.NotFound(missingItemID) }
            throw Self.uploadError("delete failed", error: error)
        }
    }

    func coordinatedRead(from url: URL) throws -> Data {
        var coordinatorError: NSError?
        var readResult: Result<Data, Error>?

        let coordinator = NSFileCoordinator()
        coordinator.coordinate(readingItemAt: url, options: [], error: &coordinatorError) {
            newURL in
            do {
                readResult = try .success(Data(contentsOf: newURL))
            } catch {
                readResult = .failure(error)
            }
        }

        if let error = coordinatorError {
            throw Self.downloadError("file coordination error", error: error)
        }

        guard let readResult else {
            throw CloudStorageError.DownloadFailed("coordinated read produced no result")
        }

        switch readResult {
        case let .success(data): return data
        case let .failure(error):
            throw Self.downloadError("read failed", error: error)
        }
    }

    /// Downloads a file from iCloud via coordinated read
    ///
    /// Tries startDownloadingUbiquitousItem as a hint, then uses NSFileCoordinator
    /// which forces the download through a different (more reliable) path
    func downloadFile(url: URL, recordId: String) throws -> Data {
        let filename = url.lastPathComponent

        try ensureDownloaded(url: url, recordId: recordId)

        let resolvedURL =
            resolvedMetadataItemIfPresent(
                named: filename,
                parentDirectoryURL: url.deletingLastPathComponent()
            )?.url ?? url

        if resolvedURL != url {
            Log.info(
                "downloadFile: using metadata URL for \(filename) local=\(url.path) metadata=\(resolvedURL.path)"
            )
        } else {
            Log.info("downloadFile: \(filename) reading via NSFileCoordinator")
        }

        return try coordinatedRead(from: resolvedURL)
    }

    // MARK: - Upload verification

    /// Blocks until the file is visible through iCloud metadata
    func waitForMetadataVisibility(url: URL) throws {
        let filename = url.lastPathComponent
        let deadline = Date().addingTimeInterval(defaultTimeout)

        do {
            let resolvedItem = try waitForMetadataItem(
                named: filename,
                parentDirectoryURL: url.deletingLastPathComponent(),
                deadline: deadline
            )
            if resolvedItem.url != url {
                Log.info(
                    "waitForMetadataVisibility: using metadata URL for \(filename) local=\(url.path) metadata=\(resolvedItem.url.path)"
                )
            }
        } catch {
            throw Self.uploadError("iCloud metadata lookup failed for \(filename)", error: error)
        }
    }

    /// Blocks until the file at `url` is confirmed uploaded to iCloud, or times out
    func waitForUpload(url: URL) throws {
        let filename = url.lastPathComponent
        Log.info("waitForUpload: waiting for \(filename)")
        let deadline = Date().addingTimeInterval(defaultTimeout)

        if case .uploaded = uploadState(for: url) {
            Log.info("waitForUpload: \(filename) already uploaded on local URL")
            return
        }

        let resolvedItem: ResolvedMetadataItem
        do {
            resolvedItem = try waitForMetadataItem(
                named: filename,
                parentDirectoryURL: url.deletingLastPathComponent(),
                deadline: deadline
            )
        } catch {
            throw Self.uploadError("iCloud metadata lookup failed for \(filename)", error: error)
        }

        if resolvedItem.url != url {
            Log.info(
                "waitForUpload: using metadata URL for \(filename) local=\(url.path) metadata=\(resolvedItem.url.path)"
            )
        }

        var lastProgressLog = Date.distantPast

        while Date() < deadline {
            let state = uploadState(for: resolvedItem.url)
            let now = Date()

            if now.timeIntervalSince(lastProgressLog) >= progressLogInterval {
                Log.info(
                    "waitForUpload: \(filename) state=\(state) metadataPath=\(resolvedItem.metadataPath ?? "<unknown>") diagnostics=\(uploadDiagnostics(for: resolvedItem.url))"
                )
                lastProgressLog = now
            }

            if case .uploaded = state {
                Log.info("waitForUpload: \(filename) uploaded")
                return
            }

            if case let .failed(error) = state {
                throw Self.uploadError("iCloud upload failed for \(filename)", error: error)
            }

            Thread.sleep(forTimeInterval: pollInterval)
        }

        Log.info(
            "waitForUpload: timeout diagnostics \(filename) metadataPath=\(resolvedItem.metadataPath ?? "<unknown>") diagnostics=\(uploadDiagnostics(for: resolvedItem.url))"
        )
        logMetadataItems(
            under: url.deletingLastPathComponent(),
            reason: "waitForUpload timeout",
            focusName: filename
        )

        throw CloudStorageError.Offline(
            "iCloud upload timed out for \(filename) after \(defaultTimeout)s"
        )
    }

    // MARK: - Download

    /// Ensures the file is downloaded locally, triggering a download if evicted
    func ensureDownloaded(url: URL, recordId: String) throws {
        // check if already downloaded
        if FileManager.default.fileExists(atPath: url.path), case .current = downloadState(for: url) {
            return
        }

        let deadline = Date().addingTimeInterval(defaultTimeout)
        let filename = url.lastPathComponent

        let resolvedItem: ResolvedMetadataItem
        do {
            resolvedItem = try waitForMetadataItem(
                named: filename,
                parentDirectoryURL: url.deletingLastPathComponent(),
                deadline: deadline
            )
        } catch {
            throw Self.downloadError("iCloud metadata lookup failed for \(filename)", error: error)
        }

        if resolvedItem.url != url {
            Log.info(
                "ensureDownloaded: using metadata URL for \(filename) local=\(url.path) metadata=\(resolvedItem.url.path)"
            )
        }

        // trigger download via startDownloadingUbiquitousItem
        do {
            try FileManager.default.startDownloadingUbiquitousItem(at: resolvedItem.url)
        } catch {
            let nsError = error as NSError
            if nsError.domain == NSCocoaErrorDomain,
               nsError.code == NSFileReadNoSuchFileError || nsError.code == 4
            {
                throw CloudStorageError.NotFound(recordId)
            }
            Log.warn("ensureDownloaded: startDownloading failed for \(filename): \(error.localizedDescription)")
        }

        // poll with periodic re-triggers — the iCloud daemon can silently
        // drop the first request on fresh installs before it's fully ready
        let retriggerInterval: TimeInterval = 5
        var lastRetrigger = Date()
        var lastProgressLog = Date.distantPast

        while Date() < deadline {
            let now = Date()

            if now.timeIntervalSince(lastRetrigger) >= retriggerInterval {
                try? FileManager.default.startDownloadingUbiquitousItem(at: resolvedItem.url)
                lastRetrigger = now
            }

            let state = downloadState(for: resolvedItem.url)

            if now.timeIntervalSince(lastProgressLog) >= progressLogInterval {
                Log.info(
                    "ensureDownloaded: \(filename) state=\(state) metadataPath=\(resolvedItem.metadataPath ?? "<unknown>")"
                )
                lastProgressLog = now
            }

            if case .current = state { return }

            if case let .failed(error) = state {
                throw Self.downloadError("iCloud download failed", error: error)
            }

            Thread.sleep(forTimeInterval: pollInterval)
        }

        // last resort: try coordinated read which forces download
        Log.info("ensureDownloaded: polling timed out, trying coordinated read for \(filename)")
        do {
            _ = try coordinatedRead(from: resolvedItem.url)
            return
        } catch {
            throw CloudStorageError.Offline(
                "iCloud download timed out after \(defaultTimeout)s (coordinated read also failed: \(error.localizedDescription))"
            )
        }
    }

    private func uploadState(for url: URL) -> UploadState {
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

    private func uploadDiagnostics(for url: URL) -> String {
        let exists = FileManager.default.fileExists(atPath: url.path)
        let fileSize: String =
            if exists,
            let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
            let size = attributes[.size] as? NSNumber {
                size.stringValue
            } else {
                "nil"
            }

        guard
            let values = try? url.resourceValues(forKeys: [
                .isUbiquitousItemKey,
                .ubiquitousItemIsUploadingKey,
                .ubiquitousItemIsUploadedKey,
                .ubiquitousItemUploadingErrorKey,
            ])
        else {
            return "exists=\(exists) fileSize=\(fileSize) values=<unavailable>"
        }

        let errorDescription = values.ubiquitousItemUploadingError?.localizedDescription ?? "nil"

        return
            "exists=\(exists) fileSize=\(fileSize) isUbiquitous=\(String(describing: values.isUbiquitousItem)) isUploading=\(String(describing: values.ubiquitousItemIsUploading)) isUploaded=\(String(describing: values.ubiquitousItemIsUploaded)) uploadingError=\(errorDescription)"
    }

    private func downloadState(for url: URL) -> DownloadState {
        guard
            let values = try? url.resourceValues(forKeys: [
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

        if let names = try? listSubdirectoriesViaFileManager(parentURL: parentURL), !names.isEmpty {
            return names.sorted()
        }

        Log.info("listSubdirectories: FileManager found nothing, falling back to metadata query")
        return try metadataSubdirectoryNames(parentDirectoryURL: parentURL)
    }

    private func listSubdirectoriesViaFileManager(parentURL: URL) throws -> [String] {
        let contents = try FileManager.default.contentsOfDirectory(
            at: parentURL, includingPropertiesForKeys: [.isDirectoryKey],
            options: []
        )

        var names = Set<String>()
        for url in contents {
            var name = url.lastPathComponent

            // iCloud evicted entries appear as .Name.icloud
            if name.hasPrefix("."), name.hasSuffix(".icloud") {
                name = String(name.dropFirst().dropLast(".icloud".count))
                names.insert(name)
                continue
            }

            if url.hasDirectoryPath {
                names.insert(name)
            }
        }

        return Array(names)
    }

    /// Lists filenames matching a prefix within a namespace directory
    ///
    /// Tries FileManager first (fast, sees .icloud stubs and dataless files),
    /// falls back to metadata query only if FileManager finds nothing
    func listFiles(namespacePath: String, prefix: String) throws -> [String] {
        let dirURL = URL(fileURLWithPath: namespacePath, isDirectory: true)

        if let names = try? listFilesViaFileManager(dirURL: dirURL, prefix: prefix), !names.isEmpty {
            return names.sorted()
        }

        Log.info("listFiles: FileManager found nothing, falling back to metadata query prefix=\(prefix)")
        return try metadataFileNames(parentDirectoryURL: dirURL, prefix: prefix)
    }

    private func listFilesViaFileManager(dirURL: URL, prefix: String) throws -> [String] {
        let contents = try FileManager.default.contentsOfDirectory(
            at: dirURL, includingPropertiesForKeys: nil,
            options: []
        )

        var names = Set<String>()
        for url in contents {
            var name = url.lastPathComponent

            // iCloud evicted files appear as .FileName.icloud
            if name.hasPrefix("."), name.hasSuffix(".icloud") {
                name = String(name.dropFirst().dropLast(".icloud".count))
            }

            if name.hasPrefix(prefix) {
                names.insert(name)
            }
        }

        return Array(names)
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

    func isBackupUploaded(namespace: String, recordId: String) throws -> Bool {
        let url = try backupFileURL(namespace: namespace, recordId: recordId)
        let resolvedURL =
            resolvedMetadataItemIfPresent(
                named: url.lastPathComponent,
                parentDirectoryURL: url.deletingLastPathComponent()
            )?.url ?? url

        let state = uploadState(for: resolvedURL)
        let usedMetadata = resolvedURL != url
        Log.info(
            "isBackupUploaded: recordId=\(recordId.prefix(12))… state=\(state) usedMetadata=\(usedMetadata)"
        )

        switch state {
        case .uploaded: return true
        default: return false
        }
    }

    private static func isNoSuchFileError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == NSCocoaErrorDomain else { return false }
        return nsError.code == NSFileNoSuchFileError || nsError.code == NSFileReadNoSuchFileError
            || nsError.code == 4
    }

    /// Checks sync health of all files in namespace directories
    func overallSyncHealth() -> SyncHealth {
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

        for nsDir in namespaceDirs where nsDir.hasDirectoryPath {
            guard
                let files = try? FileManager.default.contentsOfDirectory(
                    at: nsDir, includingPropertiesForKeys: nil
                )
            else { continue }

            for file in files where file.pathExtension == "json" {
                hasFiles = true
                let status = uploadStatus(for: file)
                switch status {
                case .uploaded: continue
                case .uploading: allUploaded = false
                case let .failed(msg):
                    anyFailed = true
                    allUploaded = false
                    failureMessage = msg
                case .unknown:
                    allUploaded = false
                }
            }
        }

        if !hasFiles { return .noFiles }
        if anyFailed { return .failed(failureMessage ?? "upload error") }
        if allUploaded { return .allUploaded }
        return .uploading
    }

    enum SyncHealth {
        case allUploaded
        case uploading
        case failed(String)
        case noFiles
        case unavailable
    }
}
