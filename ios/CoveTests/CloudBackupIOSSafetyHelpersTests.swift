@testable import Cove
import CoveCore
import XCTest

final class CloudBackupIOSSafetyHelpersTests: XCTestCase {
    func testICloudNamespaceValidationRejectsPathLikeInput() throws {
        let helper = ICloudDriveHelper.shared

        XCTAssertEqual(
            try helper.validateNamespace("0123456789abcdef0123456789abcdef"),
            "0123456789abcdef0123456789abcdef"
        )

        assertInvalidNamespace("0123456789abcdef0123456789abcdeg")
        assertInvalidNamespace("0123456789ABCDEF0123456789abcdef")
        assertInvalidNamespace("../0123456789abcdef0123456789abcd")
        assertInvalidNamespace("0123456789abcdef")
    }

    func testICloudSyncHealthOnlyScansValidNamespaceDirectories() {
        XCTAssertTrue(
            ICloudDriveHelper.isValidNamespaceDirectory(
                URL(fileURLWithPath: "/tmp/0123456789abcdef0123456789abcdef", isDirectory: true)
            )
        )
        XCTAssertFalse(
            ICloudDriveHelper.isValidNamespaceDirectory(
                URL(fileURLWithPath: "/tmp/0123456789ABCDEF0123456789abcdef", isDirectory: true)
            )
        )
        XCTAssertFalse(
            ICloudDriveHelper.isValidNamespaceDirectory(
                URL(fileURLWithPath: "/tmp/0123456789abcdef0123456789abcdef.json", isDirectory: false)
            )
        )
    }

    func testCatastrophicProbeMappingDistinguishesInconclusiveStates() {
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(result: .backupFound),
            .available
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(result: .noBackupFound(message: "no backup")),
            .noBackup
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(result: .offline(message: "offline")),
            .offline("offline")
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(result: .inconclusive(message: "icloud unavailable")),
            .inconclusive("icloud unavailable")
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(result: .inconclusive(message: "auth required")),
            .inconclusive("auth required")
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(result: .unreadable(message: "bad data")),
            .unreadable("bad data")
        )

        XCTAssertFalse(CatastrophicErrorView.CloudProbeState.inconclusive("cold metadata").allowsRestoreAttempt)
        XCTAssertFalse(CatastrophicErrorView.CloudProbeState.unreadable("bad data").allowsRestoreAttempt)
        XCTAssertTrue(CatastrophicErrorView.CloudProbeState.available.allowsRestoreAttempt)
        XCTAssertTrue(CatastrophicErrorView.CloudProbeState.offline("offline").allowsRetry)
        XCTAssertFalse(CatastrophicErrorView.CloudProbeState.offline("offline").allowsRestoreAttempt)
        XCTAssertFalse(CatastrophicErrorView.CloudProbeState.noBackup.allowsRestoreAttempt)
    }

    func testDetailHeaderUsesActiveOnlyForConfirmedUploads() {
        XCTAssertEqual(
            cloudBackupDetailHeaderTitle(syncHealth: .allUploaded),
            "Cloud Backup Active"
        )
        XCTAssertEqual(
            cloudBackupDetailHeaderIconName(syncHealth: .allUploaded),
            "checkmark.icloud.fill"
        )

        let unhealthyStates: [CloudSyncHealth] = [
            .unknown,
            .uploading,
            .noFiles,
            .authorizationRequired("auth required"),
            .unavailable,
            .failed("sync failed"),
        ]

        for state in unhealthyStates {
            XCTAssertNotEqual(cloudBackupDetailHeaderTitle(syncHealth: state), "Cloud Backup Active")
            XCTAssertNotEqual(cloudBackupDetailHeaderIconName(syncHealth: state), "checkmark.icloud.fill")
        }
    }

    func testSilentNamespaceProbeRetriesEmptyResultsUntilItFindsNamespaces() async throws {
        let state = SilentNamespaceProbeTestState(results: [[], [], ["namespace"]])

        let namespaces = try await runSilentNamespaceProbe(with: state)

        XCTAssertEqual(namespaces, ["namespace"])
        XCTAssertEqual(state.inspectionCount, 3)
        XCTAssertEqual(state.delays, [1, 2])
    }

    func testSilentNamespaceProbeStopsAfterFourEmptyInspections() async throws {
        let state = SilentNamespaceProbeTestState(results: [[], [], [], []])

        let namespaces = try await runSilentNamespaceProbe(with: state)

        XCTAssertEqual(namespaces, [])
        XCTAssertEqual(state.inspectionCount, 4)
        XCTAssertEqual(state.delays, [1, 2, 4])
        XCTAssertLessThanOrEqual(state.elapsed, SilentNamespaceRecoveryProbe.maximumDuration)
    }

    func testSilentNamespaceProbeReturnsFirstNonEmptyResultImmediately() async throws {
        let state = SilentNamespaceProbeTestState(results: [["namespace"]])

        let namespaces = try await runSilentNamespaceProbe(with: state)

        XCTAssertEqual(namespaces, ["namespace"])
        XCTAssertEqual(state.inspectionCount, 1)
        XCTAssertEqual(state.delays, [])
    }

    func testSilentNamespaceProbePreservesStorageErrors() async {
        let expectedError = CloudStorageError.Offline("iCloud unavailable")
        let state = SilentNamespaceProbeTestState(results: [], error: expectedError)

        do {
            _ = try await runSilentNamespaceProbe(with: state)
            XCTFail("expected the storage error")
        } catch let error as CloudStorageError {
            XCTAssertEqual(error, expectedError)
        } catch {
            XCTFail("unexpected error: \(error)")
        }

        XCTAssertEqual(state.inspectionCount, 1)
        XCTAssertEqual(state.delays, [])
    }

    func testSilentNamespaceProbeReservesInspectionCleanupWithinDeadline() async throws {
        let state = SilentNamespaceProbeTestState(
            results: [[], [], [], []],
            consumeInspectionBudget: true
        )

        let namespaces = try await runSilentNamespaceProbe(with: state)

        XCTAssertEqual(namespaces, [])
        XCTAssertLessThanOrEqual(state.elapsed, SilentNamespaceRecoveryProbe.maximumDuration)
        XCTAssertTrue(
            state.metadataTimeouts.allSatisfy {
                $0 <= SilentNamespaceRecoveryProbe.maximumMetadataTimeout
            }
        )
    }

    func testSilentNamespaceProbeStopsDuringRetryDelayWhenCancelled() async {
        let gate = SilentNamespaceProbeCancellationGate()
        let task = Task {
            try await SilentNamespaceRecoveryProbe.run(
                now: { 0 },
                sleep: { duration in
                    await gate.markSleepStarted(duration: duration)
                    try await Task.sleep(for: .seconds(60))
                },
                inspect: { timeout in
                    await gate.recordInspection(timeout: timeout)
                    return []
                }
            )
        }

        await gate.waitUntilSleepStarts()
        task.cancel()

        do {
            _ = try await task.value
            XCTFail("expected cancellation")
        } catch is CancellationError {
            // expected
        } catch {
            XCTFail("unexpected error: \(error)")
        }

        let inspectionCount = await gate.inspectionCount
        XCTAssertEqual(inspectionCount, 1)
    }

    func testCancellableDispatchOperationReturnsBeforeQueuedWorkFinishes() async {
        let gate = CancellableDispatchOperationTestGate()
        let queue = DispatchQueue(label: "cove.tests.cancellable-cloud-operation")
        let task = Task {
            try await CancellableDispatchOperation<[String]>.run(on: queue) {
                gate.waitUntilReleased()
                return ["late namespace"]
            }
        }

        await gate.waitUntilStarted()
        task.cancel()

        do {
            _ = try await task.value
            XCTFail("expected cancellation")
        } catch is CancellationError {
            // expected
        } catch {
            XCTFail("unexpected error: \(error)")
        }

        XCTAssertFalse(gate.didFinish)
        gate.release()
        let didFinish = await gate.waitUntilFinished()
        XCTAssertTrue(didFinish)
    }

    private func runSilentNamespaceProbe(
        with state: SilentNamespaceProbeTestState
    ) async throws -> [String] {
        try await SilentNamespaceRecoveryProbe.run(
            now: { state.now },
            sleep: { duration in try await state.sleep(for: duration) },
            inspect: { timeout in try await state.inspect(metadataTimeout: timeout) }
        )
    }

    private func assertInvalidNamespace(_ namespace: String) {
        XCTAssertThrowsError(try ICloudDriveHelper.shared.validateNamespace(namespace)) { error in
            guard case CloudStorageError.InvalidNamespace = error else {
                XCTFail("expected InvalidNamespace, got \(error)")
                return
            }
        }
    }
}

private final class SilentNamespaceProbeTestState: @unchecked Sendable {
    private let lock = NSLock()
    private var currentTime: TimeInterval = 0
    private var results: [[String]]
    private let error: CloudStorageError?
    private let consumeInspectionBudget: Bool
    private var recordedDelays: [TimeInterval] = []
    private var recordedMetadataTimeouts: [TimeInterval] = []

    init(
        results: [[String]],
        error: CloudStorageError? = nil,
        consumeInspectionBudget: Bool = false
    ) {
        self.results = results
        self.error = error
        self.consumeInspectionBudget = consumeInspectionBudget
    }

    var now: TimeInterval {
        lock.withLock { currentTime }
    }

    var elapsed: TimeInterval {
        now
    }

    var inspectionCount: Int {
        lock.withLock { recordedMetadataTimeouts.count }
    }

    var delays: [TimeInterval] {
        lock.withLock { recordedDelays }
    }

    var metadataTimeouts: [TimeInterval] {
        lock.withLock { recordedMetadataTimeouts }
    }

    func sleep(for duration: TimeInterval) async throws {
        try Task.checkCancellation()

        lock.withLock {
            recordedDelays.append(duration)
            currentTime += duration
        }
    }

    func inspect(metadataTimeout: TimeInterval) async throws -> [String] {
        try Task.checkCancellation()

        return try lock.withLock {
            recordedMetadataTimeouts.append(metadataTimeout)

            if consumeInspectionBudget {
                currentTime += metadataTimeout
                    + SilentNamespaceRecoveryProbe.metadataTimeoutCleanupAllowance
            }

            if let error {
                throw error
            }

            guard !results.isEmpty else { return [] }
            return results.removeFirst()
        }
    }
}

private actor SilentNamespaceProbeCancellationGate {
    private(set) var inspectionCount = 0
    private var sleepStarted = false

    func recordInspection(timeout _: TimeInterval) {
        inspectionCount += 1
    }

    func markSleepStarted(duration _: TimeInterval) {
        sleepStarted = true
    }

    func waitUntilSleepStarts() async {
        while !sleepStarted {
            await Task.yield()
        }
    }
}

private final class CancellableDispatchOperationTestGate: @unchecked Sendable {
    private let started = DispatchSemaphore(value: 0)
    private let releaseWork = DispatchSemaphore(value: 0)
    private let finished = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var finishedWork = false

    var didFinish: Bool {
        lock.withLock { finishedWork }
    }

    func waitUntilStarted() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                self.started.wait()
                continuation.resume()
            }
        }
    }

    func waitUntilReleased() {
        started.signal()
        releaseWork.wait()

        lock.withLock {
            finishedWork = true
        }
        finished.signal()
    }

    func release() {
        releaseWork.signal()
    }

    func waitUntilFinished() async -> Bool {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                continuation.resume(returning: self.finished.wait(timeout: .now() + 1) == .success)
            }
        }
    }
}
