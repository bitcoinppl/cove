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

    func testClearRemovesExistingLogsAndWritesMarker() {
        let store = makeStore()
        store.record(level: .warn, category: "clear", message: "before clear")
        XCTAssertTrue(store.snapshot().contains("before clear"))

        store.clear()

        let snapshot = store.snapshot()
        XCTAssertFalse(snapshot.contains("before clear"))
        XCTAssertTrue(snapshot.contains("swift diagnostics logs cleared at"))
    }

    func testSnapshotReturnsFallbackWhenNoLogsExist() {
        XCTAssertEqual(makeStore().snapshot(), "no Swift logs captured\n")
    }

    func testSnapshotAfterReinstantiatingStoreIncludesPersistedLines() {
        let store = makeStore()
        store.record(level: .error, category: "restart", message: "before restart")
        _ = store.snapshot()

        let restarted = makeStore()

        XCTAssertTrue(restarted.snapshot().contains("before restart"))
    }

    private func makeStore() -> SwiftLogStore {
        SwiftLogStore(logsDirectory: tempDirectory)
    }
}
