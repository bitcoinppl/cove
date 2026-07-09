@testable import Cove
import XCTest

final class SwiftLogStoreTests: XCTestCase {
    private var tempDirectory: URL!

    override func setUpWithError() throws {
        try super.setUpWithError()

        tempDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: tempDirectory,
            withIntermediateDirectories: true
        )
    }

    override func tearDownWithError() throws {
        if let tempDirectory {
            try? FileManager.default.removeItem(at: tempDirectory)
        }

        tempDirectory = nil
        try super.tearDownWithError()
    }

    func testRecordsAndSnapshotsMessagesWithLevelCategoryAndTimestamp() {
        let store = makeStore()

        store.record(level: .info, category: "unit", message: "hello diagnostics")

        let snapshot = store.snapshot()
        XCTAssertTrue(snapshot.split(separator: "\n").contains { line in
            String(line).range(
                of: #"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z INFO unit: hello diagnostics$"#,
                options: .regularExpression
            ) != nil
        })
    }

    func testRotationKeepsTotalBytesBoundedAndDropsOldestArchive() throws {
        let store = makeStore()
        let payload = String(repeating: "x", count: 140_000)

        for index in 0 ..< 20 {
            store.record(level: .info, category: "rotation", message: "entry-\(index) \(payload)")
        }

        let snapshot = store.snapshot()
        XCTAssertFalse(snapshot.contains("entry-0 "))
        XCTAssertTrue(snapshot.contains("entry-19 "))

        let logFiles = try FileManager.default.contentsOfDirectory(
            at: tempDirectory,
            includingPropertiesForKeys: [.fileSizeKey]
        )
        let totalBytes = try logFiles.reduce(0) { total, url in
            let values = try url.resourceValues(forKeys: [.fileSizeKey])

            return total + (values.fileSize ?? 0)
        }

        XCTAssertLessThanOrEqual(logFiles.count, 8)
        XCTAssertLessThanOrEqual(totalBytes, 2 * 1024 * 1024)
    }

    func testClearRemovesExistingLogsAndWritesMarker() throws {
        let store = makeStore()
        store.record(level: .warn, category: "clear", message: "before clear")
        XCTAssertTrue(store.snapshot().contains("before clear"))

        try store.clear()

        let snapshot = store.snapshot()
        XCTAssertFalse(snapshot.contains("before clear"))
        XCTAssertTrue(snapshot.contains("swift diagnostics logs cleared at"))
    }

    func testSnapshotReturnsFallbackWhenNoLogsExist() {
        XCTAssertEqual(makeStore().snapshot(), "no Swift logs captured\n")
    }

    func testSnapshotPreservesMultilineMessageWhitespaceForDiagnosticsRedaction() {
        let store = makeStore()
        let mnemonic = """
        abandon abandon abandon abandon abandon abandon
        abandon abandon abandon abandon abandon about
        """

        store.record(level: .warn, category: "redaction", message: mnemonic)

        let snapshot = store.snapshot()
        XCTAssertTrue(snapshot.contains("abandon abandon\nabandon abandon"))
        XCTAssertFalse(snapshot.contains("abandon abandon\\nabandon abandon"))
    }

    func testSnapshotAfterReinstantiatingStoreIncludesPersistedLines() {
        let store = makeStore()
        store.record(level: .error, category: "restart", message: "before restart")
        _ = store.snapshot()

        let restarted = makeStore()

        XCTAssertTrue(restarted.snapshot().contains("before restart"))
    }

    func testSnapshotIncludesWriteFailureBreadcrumb() throws {
        let fileURL = tempDirectory.appendingPathComponent("not-a-directory")
        try "not a directory".write(to: fileURL, atomically: true, encoding: .utf8)
        let store = SwiftLogStore(logsDirectory: fileURL)

        store.record(level: .error, category: "failure", message: "cannot write")

        let snapshot = store.snapshot()
        XCTAssertTrue(snapshot.contains("failed to write Swift diagnostics log file"))
    }

    func testClearWriteFailureIsThrownAndIncludedInSnapshot() throws {
        let fileURL = tempDirectory.appendingPathComponent("not-a-directory")
        try "not a directory".write(to: fileURL, atomically: true, encoding: .utf8)
        let store = SwiftLogStore(logsDirectory: fileURL)

        XCTAssertThrowsError(try store.clear())

        let snapshot = store.snapshot()
        XCTAssertTrue(snapshot.contains("failed to write Swift diagnostics log file"))
    }

    func testClearDeletionFailureIsThrown() {
        let fileManager = FailingRemoveFileManager()
        let store = SwiftLogStore(logsDirectory: tempDirectory, fileManager: fileManager)

        XCTAssertThrowsError(try store.clear())
    }

    private func makeStore() -> SwiftLogStore {
        SwiftLogStore(logsDirectory: tempDirectory)
    }
}

private final class FailingRemoveFileManager: FileManager, @unchecked Sendable {
    override func fileExists(atPath _: String) -> Bool {
        true
    }

    override func removeItem(at _: URL) throws {
        throw CocoaError(.fileWriteNoPermission)
    }
}
