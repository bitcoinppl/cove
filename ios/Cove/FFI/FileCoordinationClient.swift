import CoveCore
import Foundation

struct FileCoordinationClient {
    static let legacyFileReadNoSuchFileError = 4

    static func isConnectivityError(_ error: Error) -> Bool {
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

    static func uploadError(_ context: String, error: Error) -> CloudStorageError {
        if isConnectivityError(error) {
            return .Offline("\(context): \(error.localizedDescription)")
        }

        return .UploadFailed("\(context): \(error.localizedDescription)")
    }

    static func downloadError(_ context: String, error: Error) -> CloudStorageError {
        if isConnectivityError(error) {
            return .Offline("\(context): \(error.localizedDescription)")
        }

        return .DownloadFailed("\(context): \(error.localizedDescription)")
    }

    static func isNoSuchFileError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == NSCocoaErrorDomain else { return false }
        return nsError.code == NSFileNoSuchFileError
            || nsError.code == NSFileReadNoSuchFileError
            || nsError.code == Self.legacyFileReadNoSuchFileError
    }

    static func isFileReadNoSuchFileError(_ error: Error) -> Bool {
        let nsError = error as NSError
        guard nsError.domain == NSCocoaErrorDomain else { return false }
        return nsError.code == NSFileReadNoSuchFileError
            || nsError.code == Self.legacyFileReadNoSuchFileError
    }

    /// Coordinates iCloud-backed filesystem access because ubiquitous items may
    /// be placeholders, may resolve to a different concrete URL, and can fail
    /// if we touch them directly without asking the system to arbitrate first
    func createDirectory(at url: URL) throws {
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

    func write(data: Data, to url: URL) throws {
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
            try write(data: data, to: url)
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

    func delete(at url: URL, missingItemID: String) throws {
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

    func read(from url: URL) throws -> Data {
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

    func listSubdirectoriesViaFileManager(parentURL: URL) throws -> [String] {
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

    func listFilesViaFileManager(dirURL: URL, prefix: String) throws -> [String] {
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

    func uploadDiagnostics(for url: URL) -> String {
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
}
