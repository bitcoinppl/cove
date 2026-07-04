@testable import Cove
import CoveCore
import XCTest

final class ICloudDriveHelperDecompositionTests: XCTestCase {
    func testICloudPathsDerivesNamespacePaths() throws {
        let containerURL = URL(fileURLWithPath: "/tmp/Cove iCloud", isDirectory: true)
        let namespace = "0123456789abcdef0123456789abcdef"
        let paths = ICloudPaths(containerURL: containerURL)

        XCTAssertEqual(paths.dataDirectoryURL().path, "/tmp/Cove iCloud/Data")
        XCTAssertEqual(
            paths.namespacesRootURL().path,
            "/tmp/Cove iCloud/Data/\(csppNamespacesSubdirectory())"
        )
        XCTAssertEqual(
            try paths.namespaceDirectoryURL(namespace: namespace).path,
            "/tmp/Cove iCloud/Data/\(csppNamespacesSubdirectory())/\(namespace)"
        )
        XCTAssertEqual(
            try paths.walletsDirectoryURL(namespace: namespace).path,
            "/tmp/Cove iCloud/Data/\(csppNamespacesSubdirectory())/\(namespace)/\(csppWalletsDirectory())"
        )
        XCTAssertEqual(
            paths.walletLocation(filename: "wallet-1.json"),
            "\(csppWalletsDirectory())/wallet-1.json"
        )
    }

    func testICloudPathsDerivesBackupLocations() throws {
        let containerURL = URL(fileURLWithPath: "/tmp/Cove iCloud", isDirectory: true)
        let namespace = "0123456789abcdef0123456789abcdef"
        let paths = ICloudPaths(containerURL: containerURL)

        XCTAssertEqual(
            try paths.backupFileURL(
                namespace: namespace,
                location: RemoteBackupLocation(relativePath: "wallet-legacy.json")
            ).path,
            "/tmp/Cove iCloud/Data/\(csppNamespacesSubdirectory())/\(namespace)/wallet-legacy.json"
        )
        XCTAssertEqual(
            try paths.backupFileURL(
                namespace: namespace,
                location: RemoteBackupLocation(
                    relativePath: "\(csppWalletsDirectory())/wallet-current.json"
                )
            ).path,
            "/tmp/Cove iCloud/Data/\(csppNamespacesSubdirectory())/\(namespace)/\(csppWalletsDirectory())/wallet-current.json"
        )
    }

    func testMetadataVisibilityTimeoutMapsToUploadFailedWithoutSleeping() {
        let clock = TestClock(start: Date(timeIntervalSince1970: 100))
        let machine = UploadDownloadStateMachine(
            defaultTimeout: 0.5,
            pollInterval: 0.1,
            clock: clock.makeClock()
        )
        let fileURL = URL(fileURLWithPath: "/tmp/file.json")

        XCTAssertThrowsError(
            try machine.waitForMetadataVisibility(url: fileURL) { name, parentDirectoryURL, deadline in
                XCTAssertEqual(name, "file.json")
                XCTAssertEqual(parentDirectoryURL.path, "/tmp")
                XCTAssertEqual(deadline, Date(timeIntervalSince1970: 100.5))
                throw ICloudDriveHelper.MetadataLookupError.timedOut(
                    "iCloud metadata query timed out for file.json"
                )
            }
        ) { error in
            guard case let CloudStorageError.UploadFailed(message) = error else {
                XCTFail("expected UploadFailed, got \(error)")
                return
            }

            XCTAssertEqual(
                message,
                "iCloud metadata lookup failed for file.json: iCloud metadata query timed out for file.json"
            )
        }
        XCTAssertTrue(clock.sleepIntervals.isEmpty)
    }

    func testUploadCompletesAfterPollingWithInjectedClock() throws {
        let clock = TestClock(start: Date(timeIntervalSince1970: 200))
        let machine = UploadDownloadStateMachine(
            defaultTimeout: 1,
            pollInterval: 0.1,
            progressLogInterval: 10,
            clock: clock.makeClock()
        )
        let fileURL = URL(fileURLWithPath: "/tmp/file.json")
        let resolvedItem = ResolvedMetadataItem(
            url: URL(fileURLWithPath: "/tmp/metadata/file.json"),
            metadataPath: "/tmp/metadata/file.json"
        )
        let states: [UploadDownloadStateMachine.UploadState] = [
            .uploading,
            .uploading,
            .uploaded,
        ]
        var stateCallCount = 0
        var metadataLookupCount = 0
        var loggedMetadataItems = false

        try machine.waitForUpload(
            url: fileURL,
            waitForMetadataItem: { name, parentDirectoryURL, deadline in
                metadataLookupCount += 1
                XCTAssertEqual(name, "file.json")
                XCTAssertEqual(parentDirectoryURL.path, "/tmp")
                XCTAssertEqual(deadline, Date(timeIntervalSince1970: 201))
                return resolvedItem
            },
            uploadState: { url in
                XCTAssertTrue(url == fileURL || url == resolvedItem.url)
                defer { stateCallCount += 1 }
                return states[min(stateCallCount, states.count - 1)]
            },
            uploadDiagnostics: { _ in "diagnostics" },
            logMetadataItems: { _, _, _ in loggedMetadataItems = true }
        )

        XCTAssertEqual(metadataLookupCount, 1)
        XCTAssertEqual(stateCallCount, 3)
        XCTAssertEqual(clock.sleepIntervals, [0.1])
        XCTAssertFalse(loggedMetadataItems)
    }

    func testUploadStallsUntilTimeoutWithInjectedClock() {
        let clock = TestClock(start: Date(timeIntervalSince1970: 300))
        let machine = UploadDownloadStateMachine(
            defaultTimeout: 0.25,
            pollInterval: 0.1,
            progressLogInterval: 10,
            clock: clock.makeClock()
        )
        let fileURL = URL(fileURLWithPath: "/tmp/file.json")
        let resolvedItem = ResolvedMetadataItem(url: fileURL, metadataPath: "/tmp/file.json")
        var loggedMetadataParent: URL?
        var loggedMetadataReason: String?
        var loggedMetadataFocus: String?

        XCTAssertThrowsError(
            try machine.waitForUpload(
                url: fileURL,
                waitForMetadataItem: { _, _, _ in resolvedItem },
                uploadState: { _ in .uploading },
                uploadDiagnostics: { _ in "diagnostics" },
                logMetadataItems: { parentDirectoryURL, reason, focusName in
                    loggedMetadataParent = parentDirectoryURL
                    loggedMetadataReason = reason
                    loggedMetadataFocus = focusName
                }
            )
        ) { error in
            guard case let CloudStorageError.Offline(message) = error else {
                XCTFail("expected Offline, got \(error)")
                return
            }

            XCTAssertEqual(message, "iCloud upload timed out for file.json after 0.25s")
        }

        XCTAssertEqual(clock.sleepIntervals, [0.1, 0.1, 0.1])
        XCTAssertEqual(loggedMetadataParent?.path, "/tmp")
        XCTAssertEqual(loggedMetadataReason, "waitForUpload timeout")
        XCTAssertEqual(loggedMetadataFocus, "file.json")
    }
}

private final class TestClock: @unchecked Sendable {
    private(set) var sleepIntervals: [TimeInterval] = []
    private var currentDate: Date

    init(start: Date) {
        self.currentDate = start
    }

    func makeClock() -> UploadDownloadStateMachine.Clock {
        UploadDownloadStateMachine.Clock(
            now: { self.currentDate },
            sleep: { interval in
                self.sleepIntervals.append(interval)
                self.currentDate = self.currentDate.addingTimeInterval(interval)
            }
        )
    }
}
