import CoveCore
import Foundation

enum ICloudBackupLookupMode {
    case waitForSync
    case currentSnapshot
}

enum ICloudBackupReadTarget: Equatable, Sendable {
    case local(URL)
    case provider(URL, metadataPath: String?)

    var url: URL {
        switch self {
        case let .local(url), let .provider(url, _): url
        }
    }
}

enum ICloudReadAttemptDeadlineError: Error, Equatable {
    case timedOut
}

enum ICloudReadAttemptDeadline {
    private enum Outcome<Value: Sendable>: Sendable {
        case completed(Value)
        case timedOut
    }

    static func run<Value: Sendable>(
        timeout: TimeInterval,
        operation: @escaping @Sendable () async throws -> Value
    ) async throws -> Value {
        guard timeout > 0 else { throw ICloudReadAttemptDeadlineError.timedOut }

        return try await withThrowingTaskGroup(of: Outcome<Value>.self) { group in
            group.addTask {
                try await .completed(operation())
            }
            group.addTask {
                try await Task.sleep(for: .seconds(timeout))
                return .timedOut
            }

            defer { group.cancelAll() }

            guard let outcome = try await group.next() else {
                throw ICloudReadAttemptDeadlineError.timedOut
            }

            switch outcome {
            case let .completed(value):
                try Task.checkCancellation()
                return value
            case .timedOut:
                throw ICloudReadAttemptDeadlineError.timedOut
            }
        }
    }
}

final class ICloudDriveHelper: @unchecked Sendable {
    static let shared = ICloudDriveHelper()

    private static let containerIdentifier = "iCloud.com.covebitcoinwallet"
    private let dataSubdirectory = "Data"
    private let namespacesSubdirectory = csppNamespacesSubdirectory()
    private let walletsSubdirectory = csppWalletsDirectory()
    private let containerURLProvider: @Sendable () -> URL?
    let metadataIndexProvider: @MainActor @Sendable () -> ICloudMetadataIndex
    let defaultTimeout: TimeInterval
    let metadataListingTimeout: TimeInterval
    let readAttemptTimeout: TimeInterval
    private let pollInterval: TimeInterval = 0.1
    let metadataSettleInterval: TimeInterval = 0.5
    private let progressLogInterval: TimeInterval = 1
    private let fileReadQueue = DispatchQueue(
        label: "cove.ICloudDriveHelper.fileRead",
        qos: .userInitiated,
        attributes: .concurrent
    )
    private static let legacyFileReadNoSuchFileError = 4

    init(
        containerURLProvider: @escaping @Sendable () -> URL? = {
            FileManager.default.url(
                forUbiquityContainerIdentifier: ICloudDriveHelper.containerIdentifier
            )
        },
        metadataIndexProvider: @escaping @MainActor @Sendable () -> ICloudMetadataIndex = {
            ICloudMetadataIndex.shared
        },
        defaultTimeout: TimeInterval = 60,
        metadataListingTimeout: TimeInterval = 15,
        readAttemptTimeout: TimeInterval = 5
    ) {
        self.containerURLProvider = containerURLProvider
        self.metadataIndexProvider = metadataIndexProvider
        self.defaultTimeout = defaultTimeout
        self.metadataListingTimeout = metadataListingTimeout
        self.readAttemptTimeout = readAttemptTimeout
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

    private static func isConnectivityError(_ error: Error) -> Bool {
        if let urlError = error as? URLError {
            return [
                .notConnectedToInternet,
                .networkConnectionLost,
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
        guard let url = containerURLProvider() else {
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

    func validateNamespace(_ namespace: String) throws -> String {
        guard Self.isValidNamespaceID(namespace) else {
            throw CloudStorageError.InvalidNamespace("expected 32 lowercase hex characters")
        }

        return namespace
    }

    /// Directory for a specific namespace: Data/cspp-namespaces/{namespace}/
    func namespaceDirectoryURL(namespace: String) throws -> URL {
        let namespace = try validateNamespace(namespace)
        let url = try namespacesRootURL()
            .appendingPathComponent(namespace, isDirectory: true)
        try coordinatedCreateDirectory(at: url)
        return url
    }

    func namespaceDirectoryReadURL(namespace: String) throws -> URL {
        let namespace = try validateNamespace(namespace)
        return try namespacesRootReadURL()
            .appendingPathComponent(namespace, isDirectory: true)
    }

    func walletsDirectoryReadURL(namespace: String) throws -> URL {
        try namespaceDirectoryReadURL(namespace: namespace)
            .appendingPathComponent(walletsSubdirectory, isDirectory: true)
    }

    func walletLocation(filename: String) -> String {
        "\(walletsSubdirectory)/\(filename)"
    }

    func backupFileURL(namespace: String, location: RemoteBackupLocation) throws -> URL {
        let namespaceDirectory = try namespaceDirectoryURL(namespace: namespace)
        return try appendBackupLocation(location, to: namespaceDirectory, createParentDirectories: true)
    }

    func backupFileReadURL(namespace: String, location: RemoteBackupLocation) throws -> URL {
        let namespaceDirectory = try namespaceDirectoryReadURL(namespace: namespace)
        return try appendBackupLocation(location, to: namespaceDirectory, createParentDirectories: false)
    }

    func existingBackupFileReadTarget(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation],
        lookupMode: ICloudBackupLookupMode = .waitForSync
    ) async throws -> ICloudBackupReadTarget {
        let urls = try backupCandidateURLs(namespace: namespace, locations: locations)

        if let localURL = urls.first(where: { FileManager.default.fileExists(atPath: $0.path) }) {
            return .local(localURL)
        }

        guard !urls.isEmpty else { throw CloudStorageError.NotFound(recordId) }

        switch lookupMode {
        case .waitForSync:
            let match = try await waitForMetadataItems(
                matching: urls,
                timeout: defaultTimeout,
                operation: "find backup \(recordId)"
            )
            return .provider(
                match.preferred.url,
                metadataPath: match.preferred.resolvedPath
            )
        case .currentSnapshot:
            guard let match = try await metadataItemsIfPresent(matching: urls).first else {
                throw CloudStorageError.NotFound(recordId)
            }
            return .provider(match.url, metadataPath: match.metadataPath)
        }
    }

    private func backupCandidateURLs(
        namespace: String,
        locations: [RemoteBackupLocation]
    ) throws -> [URL] {
        var paths = Set<String>()

        return try locations.compactMap { location in
            let url = try backupFileReadURL(namespace: namespace, location: location)
            guard paths.insert(url.standardizedFileURL.path).inserted else { return nil }
            return url
        }
    }

    private func appendBackupLocation(
        _ location: RemoteBackupLocation,
        to namespaceDirectory: URL,
        createParentDirectories: Bool
    ) throws -> URL {
        let parts = location.relativePath.split(separator: "/").map(String.init)
        guard
            !parts.isEmpty,
            parts.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." })
        else {
            throw CloudStorageError.NotAvailable("invalid backup location")
        }

        var directory = namespaceDirectory
        for folder in parts.dropLast() {
            directory.appendPathComponent(folder, isDirectory: true)
        }

        if createParentDirectories {
            try coordinatedCreateDirectory(at: directory)
        }

        return directory.appendingPathComponent(parts.last!)
    }
}

extension ICloudDriveHelper {
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
            try validateLocallyStagedUpload(data: data, at: url)
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

        try validateLocallyStagedUpload(data: data, at: url)
    }

    private func validateLocallyStagedUpload(data: Data, at url: URL) throws {
        let stagedData = try coordinatedRead(from: url)
        guard stagedData == data else {
            throw CloudStorageError.UploadFailed("local iCloud handoff validation failed")
        }

        Log.info("writeForUpload: validated local iCloud handoff for \(url.lastPathComponent)")
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
            if Self.isNoSuchFileError(error) {
                throw CloudStorageError.NotFound(missingItemID)
            }
            throw Self.uploadError("delete failed", error: error)
        }
    }

    private static func coordinatedRead(
        from url: URL,
        coordinator: NSFileCoordinator
    ) throws -> Data {
        var coordinatorError: NSError?
        var readResult: Result<Data, Error>?

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

    func coordinatedRead(from url: URL) throws -> Data {
        try Self.coordinatedRead(from: url, coordinator: NSFileCoordinator())
    }

    private final class CoordinatedReadOperation: @unchecked Sendable {
        private let url: URL
        private let lock = NSLock()
        private var coordinator: NSFileCoordinator?
        private var isCancelled = false

        init(url: URL) {
            self.url = url
        }

        func run() throws -> Data {
            let coordinator = NSFileCoordinator()
            let shouldCancel = lock.withLock {
                self.coordinator = coordinator
                return isCancelled
            }

            if shouldCancel {
                coordinator.cancel()
            }

            defer {
                lock.withLock {
                    self.coordinator = nil
                }
            }

            return try ICloudDriveHelper.coordinatedRead(
                from: url,
                coordinator: coordinator
            )
        }

        func cancel() {
            let activeCoordinator = lock.withLock {
                isCancelled = true
                return self.coordinator
            }
            activeCoordinator?.cancel()
        }
    }

    private func coordinatedReadAsync(from url: URL) async throws -> Data {
        let operation = CoordinatedReadOperation(url: url)

        return try await withTaskCancellationHandler {
            try await CancellableDispatchOperation.run(on: fileReadQueue) {
                try operation.run()
            }
        } onCancel: {
            operation.cancel()
        }
    }

    private func directReadAsync(from url: URL) async throws -> Data {
        try await CancellableDispatchOperation.run(on: fileReadQueue) {
            try Data(contentsOf: url)
        }
    }

    /// Reads a local backup directly or materializes a provider item with bounded coordination
    func downloadFile(target: ICloudBackupReadTarget, recordId: String) async throws -> Data {
        try await downloadData(target: target, recordId: recordId)
    }

    // MARK: - Download

    private func downloadData(
        target: ICloudBackupReadTarget,
        recordId: String
    ) async throws -> Data {
        let targetURL = target.url
        let filename = targetURL.lastPathComponent
        let deadline = Date().addingTimeInterval(defaultTimeout)
        let resolvedItem: ResolvedMetadataItem

        switch target {
        case let .local(url):
            Log.info("downloadFile: reading locally visible file directly for \(filename)")

            do {
                return try await boundedDirectRead(
                    from: url,
                    filename: filename,
                    deadline: deadline
                )
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                Log.warn(
                    "downloadFile: direct local read failed for \(filename): \(error.localizedDescription)"
                )
                resolvedItem = try await resolveDownloadItem(
                    url: url,
                    filename: filename,
                    deadline: deadline
                )
            }
        case let .provider(url, metadataPath):
            resolvedItem = ResolvedMetadataItem(url: url, metadataPath: metadataPath)
        }

        logResolvedDownloadItem(resolvedItem, targetURL: targetURL, filename: filename)
        try triggerDownload(url: resolvedItem.url, recordId: recordId, filename: filename)

        var readState = CoordinatedReadState()
        switch try await attemptCoordinatedRead(
            from: resolvedItem.url,
            filename: filename,
            reason: "initial",
            attempt: readState.attempt,
            deadline: deadline
        ) {
        case let .success(data):
            return data
        case let .failure(error):
            readState.lastError = error
        }

        return try await pollForDownload(
            resolvedItem: resolvedItem,
            filename: filename,
            recordId: recordId,
            deadline: deadline,
            readState: readState
        )
    }

    private func resolveDownloadItem(
        url: URL,
        filename: String,
        deadline: Date
    ) async throws -> ResolvedMetadataItem {
        do {
            return try await waitForMetadataItem(
                named: filename,
                parentDirectoryURL: url.deletingLastPathComponent(),
                deadline: deadline
            )
        } catch is CancellationError {
            throw CancellationError()
        } catch let error as CloudStorageError {
            throw error
        } catch {
            throw Self.downloadError("iCloud metadata lookup failed for \(filename)", error: error)
        }
    }

    private func logResolvedDownloadItem(
        _ resolvedItem: ResolvedMetadataItem,
        targetURL: URL,
        filename: String
    ) {
        if resolvedItem.url != targetURL {
            Log.info(
                "downloadFile: using metadata URL for \(filename) target=\(targetURL.path) metadata=\(resolvedItem.url.path)"
            )
        } else {
            Log.info("downloadFile: \(filename) reading via provider URL")
        }
    }

    private func boundedDirectRead(
        from url: URL,
        filename: String,
        deadline: Date
    ) async throws -> Data {
        let timeout = try readAttemptDuration(filename: filename, deadline: deadline)

        do {
            return try await ICloudReadAttemptDeadline.run(timeout: timeout) {
                try await self.directReadAsync(from: url)
            }
        } catch is CancellationError {
            throw CancellationError()
        } catch ICloudReadAttemptDeadlineError.timedOut {
            throw CloudStorageError.SyncPending(
                "iCloud is still downloading \(filename)"
            )
        } catch let error as CloudStorageError {
            throw error
        } catch {
            throw Self.downloadError("direct read failed for \(filename)", error: error)
        }
    }

    private func boundedCoordinatedRead(
        from url: URL,
        filename: String,
        deadline: Date
    ) async throws -> Data {
        let timeout = try readAttemptDuration(filename: filename, deadline: deadline)

        do {
            return try await ICloudReadAttemptDeadline.run(timeout: timeout) {
                try await self.coordinatedReadAsync(from: url)
            }
        } catch is CancellationError {
            throw CancellationError()
        } catch ICloudReadAttemptDeadlineError.timedOut {
            throw CloudStorageError.SyncPending(
                "iCloud is still downloading \(filename)"
            )
        } catch let error as CloudStorageError {
            throw error
        } catch {
            throw Self.downloadError("coordinated read failed for \(filename)", error: error)
        }
    }

    private func readAttemptDuration(filename: String, deadline: Date) throws -> TimeInterval {
        let remaining = deadline.timeIntervalSinceNow
        guard remaining > 0 else {
            throw CloudStorageError.SyncPending("iCloud is still downloading \(filename)")
        }

        return min(readAttemptTimeout, remaining)
    }

    private func attemptCoordinatedRead(
        from url: URL,
        filename: String,
        reason: String,
        attempt: Int,
        deadline: Date
    ) async throws -> Result<Data, Error> {
        Log.info(
            "downloadFile: trying coordinated read attempt=\(attempt) reason=\(reason) file=\(filename)"
        )

        do {
            let data = try await boundedCoordinatedRead(
                from: url,
                filename: filename,
                deadline: deadline
            )
            Log.info("downloadFile: coordinated read succeeded for \(filename)")
            return .success(data)
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            Log.warn(
                "downloadFile: coordinated read failed attempt=\(attempt): \(error.localizedDescription)"
            )

            return .failure(error)
        }
    }

    private func pollForDownload(
        resolvedItem: ResolvedMetadataItem,
        filename: String,
        recordId: String,
        deadline: Date,
        readState initialReadState: CoordinatedReadState
    ) async throws -> Data {
        // poll with periodic re-triggers and inline coordinated reads because
        // some restored-device placeholders never transition out of
        // not-downloaded even though they can be materialized
        let retriggerInterval: TimeInterval = 5
        var lastRetrigger = Date()
        var lastDirectRead = Date.distantPast
        var lastProgressLog = Date.distantPast
        var readState = initialReadState

        while Date() < deadline {
            let now = Date()
            let state = downloadState(for: resolvedItem.url)

            if now.timeIntervalSince(lastRetrigger) >= retriggerInterval {
                try? triggerDownload(url: resolvedItem.url, recordId: recordId, filename: filename)
                lastRetrigger = now
                readState.attempt += 1

                switch try await attemptCoordinatedRead(
                    from: resolvedItem.url,
                    filename: filename,
                    reason: "retry",
                    attempt: readState.attempt,
                    deadline: deadline
                ) {
                case let .success(data):
                    return data
                case let .failure(error):
                    readState.lastError = error
                }
            }

            if now.timeIntervalSince(lastProgressLog) >= progressLogInterval {
                Log.info(
                    "downloadFile: \(filename) state=\(state) metadataPath=\(resolvedItem.metadataPath ?? "<unknown>")"
                )
                lastProgressLog = now
            }

            if case .current = state,
               now.timeIntervalSince(lastDirectRead) >= retriggerInterval
            {
                lastDirectRead = now
                Log.info("downloadFile: provider reports current for \(filename)")

                do {
                    return try await boundedDirectRead(
                        from: resolvedItem.url,
                        filename: filename,
                        deadline: deadline
                    )
                } catch is CancellationError {
                    throw CancellationError()
                } catch {
                    readState.lastError = error
                    Log.warn(
                        "downloadFile: direct provider read failed for \(filename): \(error.localizedDescription)"
                    )
                }
            }

            if case let .failed(error) = state {
                throw Self.downloadError("iCloud download failed", error: error)
            }

            try await Task.sleep(for: .seconds(pollInterval))
        }

        let diagnosticError = readState.lastError?.localizedDescription ?? "none"
        throw CloudStorageError.SyncPending(
            "iCloud is still downloading the requested file (last read error: \(diagnosticError))"
        )
    }

    private struct CoordinatedReadState {
        var attempt = 1
        var lastError: Error?
    }

    private func triggerDownload(url: URL, recordId: String, filename _: String) throws {
        do {
            try FileManager.default.startDownloadingUbiquitousItem(at: url)
        } catch {
            let nsError = error as NSError
            if nsError.domain == NSCocoaErrorDomain,
               nsError.code == NSFileReadNoSuchFileError
               // some iCloud missing-file cases surface as legacy Cocoa code 4
               || nsError.code == Self.legacyFileReadNoSuchFileError
            {
                throw CloudStorageError.NotFound(recordId)
            }
            Log.warn("downloadFile: startDownloading failed: \(error.localizedDescription)")
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
        if values.ubiquitousItemIsUploaded == true {
            return .uploaded
        }
        if let error = values.ubiquitousItemUploadingError {
            return .failed(error)
        }

        return .uploading
    }

    private func downloadState(for url: URL) -> DownloadState {
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
        if values.ubiquitousItemDownloadingStatus == .current {
            return .current
        }
        if let error = values.ubiquitousItemDownloadingError {
            return .failed(error)
        }
        if values.ubiquitousItemIsDownloading == true {
            return .downloading
        }

        return .notDownloaded
    }

    func locallyVisibleSubdirectoryNames(parentPath: String) -> [String] {
        let parentURL = URL(fileURLWithPath: parentPath, isDirectory: true)
        return (try? listSubdirectoriesViaFileManager(parentURL: parentURL).sorted()) ?? []
    }

    func metadataSubdirectoryNames(
        parentPath: String,
        timeout: TimeInterval? = nil
    ) async throws -> [String] {
        try await metadataSubdirectoryNames(
            parentDirectoryURL: URL(fileURLWithPath: parentPath, isDirectory: true),
            timeout: timeout ?? metadataListingTimeout
        )
    }

    func locallyVisibleFileNames(namespacePath: String, prefix: String) -> [String] {
        let directoryURL = URL(fileURLWithPath: namespacePath, isDirectory: true)
        return (try? listFilesViaFileManager(dirURL: directoryURL, prefix: prefix).sorted()) ?? []
    }

    func localFileSnapshot(namespacePath: String, prefix: String) throws -> [String] {
        let directoryURL = URL(fileURLWithPath: namespacePath, isDirectory: true)
        return try listFilesViaFileManager(dirURL: directoryURL, prefix: prefix).sorted()
    }

    func metadataFileNames(namespacePath: String, prefix: String) async throws -> [String] {
        try await metadataFileNames(
            parentDirectoryURL: URL(fileURLWithPath: namespacePath, isDirectory: true),
            prefix: prefix
        )
    }

    private func listSubdirectoriesViaFileManager(parentURL: URL) throws -> [String] {
        let contents = try FileManager.default.contentsOfDirectory(
            at: parentURL,
            includingPropertiesForKeys: [.isDirectoryKey],
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

    func isBackupUploaded(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation]
    ) async throws -> CloudBackupUploadStatus {
        let urls = try locations.map { location in
            try backupFileReadURL(namespace: namespace, location: location)
        }
        var foundPendingItem = false

        for url in urls {
            let metadataItem = try await metadataItemIfPresent(
                named: url.lastPathComponent,
                parentDirectoryURL: url.deletingLastPathComponent()
            )
            let resolvedURL = metadataItem?.url ?? url
            let exists = FileManager.default.fileExists(atPath: resolvedURL.path)
            guard metadataItem != nil || exists else { continue }

            let state = uploadState(for: resolvedURL)
            let usedMetadata = resolvedURL != url
            Log.info(
                "isBackupUploaded: recordId=\(recordId.prefix(12))… path=\(url.path) state=\(state) usedMetadata=\(usedMetadata)"
            )

            if case .uploaded = state {
                return .uploaded
            }

            foundPendingItem = true
        }

        return foundPendingItem ? .pending : .notFound
    }

    func deleteExistingBackupFile(
        namespace: String,
        recordId: String,
        locations: [RemoteBackupLocation]
    ) async throws {
        let candidateURLs = try backupCandidateURLs(namespace: namespace, locations: locations)
        guard !candidateURLs.isEmpty else { throw CloudStorageError.NotFound(recordId) }

        let localURLs = candidateURLs.filter { FileManager.default.fileExists(atPath: $0.path) }
        let metadataURLs: [URL]
        if localURLs.isEmpty {
            let lateMatch = try await waitForMetadataItems(
                matching: candidateURLs,
                timeout: defaultTimeout,
                operation: "delete backup \(recordId)"
            )
            let cleanupItems = await visibleMetadataItems(matching: candidateURLs)
            metadataURLs = lateMatch.records.map(\.url) + cleanupItems.map(\.url)
        } else {
            metadataURLs = try await metadataItemsIfPresent(matching: candidateURLs).map(\.url)
        }

        var seenPaths = Set<String>()
        let urlsToDelete = (localURLs + metadataURLs).filter { url in
            seenPaths.insert(url.standardizedFileURL.path).inserted
        }
        var deletedAny = false
        var lastError: Error?

        for url in urlsToDelete {
            try Task.checkCancellation()

            do {
                try coordinatedDelete(at: url, missingItemID: recordId)
                deletedAny = true
            } catch CloudStorageError.NotFound {
                continue
            } catch {
                lastError = error
            }
        }

        if let lastError {
            throw lastError
        }
        guard deletedAny else { throw CloudStorageError.NotFound(recordId) }
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

    private func hasUploadedState(for file: URL) -> (hasFile: Bool, allUploaded: Bool, failed: Bool) {
        let status = uploadStatus(for: file)
        switch status {
        case .uploaded:
            return (true, true, false)
        case .uploading, .unknown:
            return (true, false, false)
        case .failed:
            return (true, false, true)
        }
    }

    private static func isNoSuchFileError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == NSCocoaErrorDomain else { return false }
        return nsError.code == NSFileNoSuchFileError
            || nsError.code == NSFileReadNoSuchFileError
            || nsError.code == Self.legacyFileReadNoSuchFileError
    }

    private static func isValidNamespaceID(_ namespace: String) -> Bool {
        namespace.count == 32 && namespace.utf8.allSatisfy { byte in
            switch byte {
            case 48 ... 57, 97 ... 102:
                true
            default:
                false
            }
        }
    }

    static func isValidNamespaceDirectory(_ url: URL) -> Bool {
        url.hasDirectoryPath && isValidNamespaceID(url.lastPathComponent)
    }

    static func syncHealth(hasFiles: Bool, allUploaded: Bool, anyFailed: Bool) -> CloudSyncHealth {
        if !hasFiles { return .noFiles }
        if anyFailed { return .failed("Some backups couldn't finish syncing to iCloud. Please try again.") }
        if allUploaded { return .allUploaded }
        return .uploading
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

        for nsDir in namespaceDirs where Self.isValidNamespaceDirectory(nsDir) {
            for file in allBackupFiles(in: nsDir) {
                let state = hasUploadedState(for: file)
                hasFiles = hasFiles || state.hasFile
                allUploaded = allUploaded && state.allUploaded
                anyFailed = anyFailed || state.failed
            }
        }

        return Self.syncHealth(
            hasFiles: hasFiles,
            allUploaded: allUploaded,
            anyFailed: anyFailed
        )
    }
}
