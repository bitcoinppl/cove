import CoveCore
import Foundation

struct ResolvedMetadataItem {
    let url: URL
    let metadataPath: String?
}

struct UploadDownloadStateMachine {
    enum UploadState: CustomStringConvertible {
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

    enum DownloadState: CustomStringConvertible {
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

    struct Clock {
        let now: @Sendable () -> Date
        let sleep: @Sendable (TimeInterval) -> Void

        static let system = Clock(
            now: { Date() },
            sleep: { interval in Thread.sleep(forTimeInterval: interval) }
        )
    }

    let defaultTimeout: TimeInterval
    let pollInterval: TimeInterval
    let progressLogInterval: TimeInterval
    let clock: Clock

    init(
        defaultTimeout: TimeInterval = 60,
        pollInterval: TimeInterval = 0.1,
        progressLogInterval: TimeInterval = 1,
        clock: Clock = .system
    ) {
        self.defaultTimeout = defaultTimeout
        self.pollInterval = pollInterval
        self.progressLogInterval = progressLogInterval
        self.clock = clock
    }

    /// Blocks until the file is visible through iCloud metadata
    func waitForMetadataVisibility(
        url: URL,
        waitForMetadataItem: (String, URL, Date) throws -> ResolvedMetadataItem
    ) throws {
        let filename = url.lastPathComponent
        let deadline = clock.now().addingTimeInterval(defaultTimeout)

        do {
            let resolvedItem = try waitForMetadataItem(
                filename,
                url.deletingLastPathComponent(),
                deadline
            )
            if resolvedItem.url != url {
                Log.info(
                    "waitForMetadataVisibility: using metadata URL for \(filename) local=\(url.path) metadata=\(resolvedItem.url.path)"
                )
            }
        } catch {
            throw FileCoordinationClient.uploadError(
                "iCloud metadata lookup failed for \(filename)",
                error: error
            )
        }
    }

    /// Blocks until the file at `url` is confirmed uploaded to iCloud, or times out
    func waitForUpload(
        url: URL,
        waitForMetadataItem: (String, URL, Date) throws -> ResolvedMetadataItem,
        uploadState: (URL) -> UploadState,
        uploadDiagnostics: (URL) -> String,
        logMetadataItems: (URL, String, String) -> Void
    ) throws {
        let filename = url.lastPathComponent
        Log.info("waitForUpload: waiting for \(filename)")
        let deadline = clock.now().addingTimeInterval(defaultTimeout)

        if case .uploaded = uploadState(url) {
            Log.info("waitForUpload: \(filename) already uploaded on local URL")
            return
        }

        let resolvedItem: ResolvedMetadataItem
        do {
            resolvedItem = try waitForMetadataItem(
                filename,
                url.deletingLastPathComponent(),
                deadline
            )
        } catch {
            throw FileCoordinationClient.uploadError(
                "iCloud metadata lookup failed for \(filename)",
                error: error
            )
        }

        if resolvedItem.url != url {
            Log.info(
                "waitForUpload: using metadata URL for \(filename) local=\(url.path) metadata=\(resolvedItem.url.path)"
            )
        }

        var lastProgressLog = Date.distantPast

        while clock.now() < deadline {
            let state = uploadState(resolvedItem.url)
            let now = clock.now()

            if now.timeIntervalSince(lastProgressLog) >= progressLogInterval {
                Log.info(
                    "waitForUpload: \(filename) state=\(state) metadataPath=\(resolvedItem.metadataPath ?? "<unknown>") diagnostics=\(uploadDiagnostics(resolvedItem.url))"
                )
                lastProgressLog = now
            }

            if case .uploaded = state {
                Log.info("waitForUpload: \(filename) uploaded")
                return
            }

            if case let .failed(error) = state {
                throw FileCoordinationClient.uploadError(
                    "iCloud upload failed for \(filename)",
                    error: error
                )
            }

            clock.sleep(pollInterval)
        }

        Log.info(
            "waitForUpload: timeout diagnostics \(filename) metadataPath=\(resolvedItem.metadataPath ?? "<unknown>") diagnostics=\(uploadDiagnostics(resolvedItem.url))"
        )
        logMetadataItems(
            url.deletingLastPathComponent(),
            "waitForUpload timeout",
            filename
        )

        throw CloudStorageError.Offline(
            "iCloud upload timed out for \(filename) after \(defaultTimeout)s"
        )
    }

    func downloadFile(
        url: URL,
        recordId: String,
        fileExists: (URL) -> Bool,
        downloadState: (URL) -> DownloadState,
        waitForMetadataItem: (String, URL, Date) throws -> ResolvedMetadataItem,
        triggerDownload: (URL, String, String) throws -> Void,
        coordinatedRead: (URL) throws -> Data
    ) throws -> Data {
        let filename = url.lastPathComponent

        if fileExists(url), case .current = downloadState(url) {
            Log.info("downloadFile: \(filename) already current on local URL")
            return try coordinatedRead(url)
        }

        let deadline = clock.now().addingTimeInterval(defaultTimeout)

        let resolvedItem: ResolvedMetadataItem
        do {
            resolvedItem = try waitForMetadataItem(
                filename,
                url.deletingLastPathComponent(),
                deadline
            )
        } catch {
            throw FileCoordinationClient.downloadError(
                "iCloud metadata lookup failed for \(filename)",
                error: error
            )
        }

        if resolvedItem.url != url {
            Log.info(
                "downloadFile: using metadata URL for \(filename) local=\(url.path) metadata=\(resolvedItem.url.path)"
            )
        } else {
            Log.info("downloadFile: \(filename) reading via resolved local URL")
        }

        try triggerDownload(resolvedItem.url, recordId, filename)

        var coordinatedReadAttempt = 1
        var lastCoordinatedReadError: Error?

        // try a coordinated read before waiting for iCloud status to catch up
        Log.info(
            "downloadFile: trying coordinated read attempt=\(coordinatedReadAttempt) reason=initial file=\(filename)"
        )

        do {
            let data = try coordinatedRead(resolvedItem.url)
            Log.info("downloadFile: coordinated read succeeded for \(filename)")
            return data
        } catch {
            lastCoordinatedReadError = error
            Log.warn(
                "downloadFile: coordinated read failed attempt=\(coordinatedReadAttempt): \(error.localizedDescription)"
            )
        }

        // poll with periodic re-triggers and inline coordinated reads because
        // some restored-device placeholders never transition out of
        // not-downloaded even though they can be materialized
        let retriggerInterval: TimeInterval = 5
        var lastRetrigger = clock.now()
        var lastProgressLog = Date.distantPast

        while clock.now() < deadline {
            let now = clock.now()
            let state = downloadState(resolvedItem.url)

            if now.timeIntervalSince(lastRetrigger) >= retriggerInterval {
                try? triggerDownload(resolvedItem.url, recordId, filename)
                lastRetrigger = now
                coordinatedReadAttempt += 1

                Log.info(
                    "downloadFile: trying coordinated read attempt=\(coordinatedReadAttempt) reason=retry file=\(filename)"
                )

                do {
                    let data = try coordinatedRead(resolvedItem.url)
                    Log.info("downloadFile: coordinated read succeeded for \(filename)")
                    return data
                } catch {
                    lastCoordinatedReadError = error
                    Log.warn(
                        "downloadFile: coordinated read failed attempt=\(coordinatedReadAttempt): \(error.localizedDescription)"
                    )
                }
            }

            if now.timeIntervalSince(lastProgressLog) >= progressLogInterval {
                Log.info(
                    "downloadFile: \(filename) state=\(state) metadataPath=\(resolvedItem.metadataPath ?? "<unknown>")"
                )
                lastProgressLog = now
            }

            if case .current = state {
                Log.info("downloadFile: poll path won for \(filename)")
                return try coordinatedRead(resolvedItem.url)
            }

            if case let .failed(error) = state {
                throw FileCoordinationClient.downloadError("iCloud download failed", error: error)
            }

            clock.sleep(pollInterval)
        }

        Log.info("downloadFile: polling timed out, trying final coordinated read for \(filename)")
        do {
            return try coordinatedRead(resolvedItem.url)
        } catch {
            let diagnosticError = lastCoordinatedReadError?.localizedDescription ?? "none"
            throw CloudStorageError.Offline(
                "iCloud download timed out after \(defaultTimeout)s (last coordinated read error: \(diagnosticError), final coordinated read failed: \(error.localizedDescription))"
            )
        }
    }
}
