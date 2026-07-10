@testable import Cove
import CoveCore
import XCTest

final class CloudBackupIOSSafetyHelpersTests: XCTestCase {
    func testPendingUploadAccessibilityStatusDistinguishesActionableStates() {
        XCTAssertEqual(
            cloudBackupPendingUploadAccessibilityStatus(
                verificationState: .awaitingUploadConfirmation,
                syncState: .blocked("iCloud authorization required")
            ),
            .authorizationRequired
        )
        XCTAssertEqual(
            cloudBackupPendingUploadAccessibilityStatus(
                verificationState: .awaitingUploadConfirmation,
                syncState: .failed("upload confirmation failed")
            ),
            .failed
        )
        XCTAssertEqual(
            cloudBackupPendingUploadAccessibilityStatus(
                verificationState: .awaitingUploadConfirmation,
                syncState: .syncing
            ),
            .confirming
        )
        XCTAssertEqual(
            cloudBackupPendingUploadAccessibilityStatus(
                verificationState: .required,
                syncState: .failed("not a pending upload")
            ),
            .hidden
        )
    }

    func testWalletAccessibilityLabelCombinesIdentityStatusAndAction() {
        let item = CloudBackupWalletItem(
            name: "Savings",
            network: .bitcoin,
            walletMode: nil,
            walletType: nil,
            fingerprint: nil,
            labelCount: nil,
            backupUpdatedAt: nil,
            syncStatus: .unsupportedVersion,
            restoreFailure: CloudBackupWalletRestoreFailure(
                message: "This wallet could not be restored. Try again."
            ),
            recordId: "wallet-record"
        )

        let label = cloudBackupWalletAccessibilityLabel(
            item: item,
            action: "Restore requires a newer version of Cove; delete is available"
        )

        XCTAssertTrue(label.contains("Savings"))
        XCTAssertTrue(label.contains("Bitcoin"))
        XCTAssertTrue(label.contains("Unsupported"))
        XCTAssertTrue(label.contains("newer version of Cove"))
        XCTAssertTrue(label.contains("delete is available"))
        XCTAssertTrue(label.contains("Restore failed"))
        XCTAssertTrue(label.contains("Try again"))
    }

    func testRestoreAllPresentationUsesRustOwnedAvailabilityAndCount() {
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .notShown),
            .hidden
        )
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .startAvailable(walletCount: 3)),
            .action(CloudBackupRestoreAllActionPresentation(
                title: "Restore All (3)",
                intent: .start
            ))
        )
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .startDisabled(walletCount: 3)),
            .disabled(title: "Restore All (3)")
        )
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .retryAvailable(walletCount: 1)),
            .action(CloudBackupRestoreAllActionPresentation(
                title: "Retry Remaining (1)",
                intent: .retry
            ))
        )
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .retryDisabled(walletCount: 1)),
            .disabled(title: "Retry Remaining (1)")
        )
    }

    func testRestoreAllProgressCopyIncludesCurrentWalletCountsAndCancellation() {
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .running(
                completed: 1,
                total: 3,
                currentWalletName: "Savings",
                cancellationRequested: false
            )),
            .running(CloudBackupRestoreAllProgressPresentation(
                completed: 1,
                total: 3,
                title: "Restoring Savings",
                detail: "Completed 1 of 3",
                accessibilityValue: "Restoring Savings, Completed 1 of 3",
                canCancel: true
            ))
        )

        let cancelling = cloudBackupRestoreAllPresentation(state: .running(
            completed: 1,
            total: 3,
            currentWalletName: "Savings",
            cancellationRequested: true
        ))
        guard case let .running(progress) = cancelling else {
            return XCTFail("expected running presentation")
        }

        XCTAssertFalse(progress.canCancel)
        XCTAssertTrue(progress.accessibilityValue.contains("Cancel requested"))
        XCTAssertTrue(progress.accessibilityValue.contains("current wallet will finish"))
    }

    func testEnableBusyCopyProjectsUploadCountsForInitialAndRetryFlows() {
        let progress = CloudBackupProgress(completed: 2, total: 5)
        let hidden = CloudBackupVerificationPresentation.hidden(source: nil)

        for flow in [
            CloudBackupEnableFlow.uploadingInitialBackup(progress: progress),
            CloudBackupEnableFlow.retryingUploadWithStagedMaterial(progress: progress),
        ] {
            let copy = cloudBackupEnableBusyCopy(
                enableFlow: flow,
                verificationPresentation: hidden
            )

            XCTAssertEqual(copy.title, "Creating your encrypted backup...")
            XCTAssertEqual(copy.subtitle, "Completed 2 of 5")
            XCTAssertEqual(copy.progress, progress)
        }
    }

    func testEnableBusyCopyPreservesPhaseAndBackgroundConfirmationCopy() {
        let hidden = CloudBackupVerificationPresentation.hidden(source: nil)
        XCTAssertEqual(
            cloudBackupEnableBusyCopy(
                enableFlow: .confirmingSavedPasskey,
                verificationPresentation: hidden
            ).title,
            "Confirming your passkey..."
        )
        XCTAssertEqual(
            cloudBackupEnableBusyCopy(
                enableFlow: .uploadingInitialBackup(progress: nil),
                verificationPresentation: hidden
            ).subtitle,
            "Cloud Backup will continue automatically"
        )

        let background = cloudBackupEnableBusyCopy(
            enableFlow: nil,
            verificationPresentation: .backgroundConfirming(.onboarding)
        )
        XCTAssertEqual(background.title, "Confirming your encrypted backup...")
        XCTAssertTrue(background.subtitle.contains("visible in iCloud"))
        XCTAssertTrue(background.subtitle.contains("continues in the background"))
        XCTAssertNil(background.progress)
    }

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

    func testICloudSyncHealthDoesNotPublishProviderDiagnostics() {
        XCTAssertEqual(
            ICloudDriveHelper.syncHealth(
                hasFiles: true,
                allUploaded: false,
                anyFailed: true
            ),
            .failed("Some backups couldn't finish syncing to iCloud. Please try again.")
        )
    }

    func testICloudInventoryAlwaysUnionsLocalAndAuthoritativeNames() throws {
        var queriedAuthoritativeInventory = false

        let names = try ICloudInventoryUnion.load {
            ["bbbb", "aaaa"]
        } authoritativeInventory: {
            queriedAuthoritativeInventory = true
            return ["aaaa", "cccc"]
        }

        XCTAssertTrue(queriedAuthoritativeInventory)
        XCTAssertEqual(names, ["aaaa", "bbbb", "cccc"])
    }

    func testICloudInventoryKeepsLocalNamesAfterEmptyAuthoritativeResult() throws {
        var queriedAuthoritativeInventory = false

        let names = try ICloudInventoryUnion.load {
            ["aaaa"]
        } authoritativeInventory: {
            queriedAuthoritativeInventory = true
            return []
        }

        XCTAssertTrue(queriedAuthoritativeInventory)
        XCTAssertEqual(names, ["aaaa"])
    }

    func testICloudInventoryDoesNotPromoteLocalNamesWhenAuthoritativeListingFails() {
        XCTAssertThrowsError(
            try ICloudInventoryUnion.load {
                ["aaaa"]
            } authoritativeInventory: {
                throw CloudStorageError.Offline("metadata query timed out")
            }
        )
    }

    func testICloudInventoryNormalizesAndDeduplicatesEvictedStubs() {
        XCTAssertEqual(
            ICloudInventoryUnion.normalizedNames([".aaaa.icloud", "aaaa", "bbbb"]),
            ["aaaa", "bbbb"]
        )
    }

    func testMetadataSettleSchedulerCoalescesUpdatesUntilTheLatestWindow() {
        var enqueuedWork: [DispatchWorkItem] = []
        var notifications: [String] = []
        let scheduler = MetadataSettleScheduler { _, workItem in
            enqueuedWork.append(workItem)
        }

        scheduler.schedule(after: 0.5) {
            notifications.append("stale")
        }
        scheduler.schedule(after: 0.5) {
            notifications.append("latest")
        }
        enqueuedWork.forEach { $0.perform() }

        XCTAssertEqual(notifications, ["latest"])
    }

    func testCloudBackupDetailStateRetainsRowsButOnlyCompleteEnablesActions() {
        let detail = CloudBackupDetail(
            lastSync: nil,
            upToDate: [],
            needsSync: [],
            cloudOnlyCount: 0,
            otherBackups: .loaded(summary: CloudBackupOtherBackupsSummary(
                namespaceCount: 0,
                walletCount: 0,
                passkeyHints: []
            ))
        )
        let loaded = LoadedCloudBackupDetail(
            detail: detail,
            cloudOnly: .notFetched,
            cloudOnlyOperation: .idle,
            otherBackupsOperation: .idle
        )

        let checking = CloudBackupDetailState.checking(retained: loaded)
        XCTAssertTrue(checking.isChecking)
        XCTAssertFalse(checking.isComplete)
        XCTAssertEqual(checking.retainedDetailState?.detail, detail)

        let failed = CloudBackupDetailState.failed(
            reason: .offline,
            error: "iCloud inventory is unavailable",
            retained: loaded
        )
        XCTAssertFalse(failed.isComplete)
        XCTAssertEqual(failed.inventoryError, "iCloud inventory is unavailable")
        XCTAssertEqual(failed.retainedDetailState?.detail, detail)

        let complete = CloudBackupDetailState.complete(state: loaded)
        XCTAssertTrue(complete.isComplete)
        XCTAssertNil(complete.inventoryError)
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
        XCTAssertTrue(
            state.metadataTimeouts.allSatisfy {
                $0 <= SilentNamespaceRecoveryProbe.maximumMetadataTimeout
            }
        )
    }

    func testSilentNamespaceProbeReportsDeadlineExhaustionAsUnavailable() async {
        let state = SilentNamespaceProbeTestState(
            results: [[], [], []],
            inspectionDurations: [4, 4, 3]
        )

        do {
            _ = try await runSilentNamespaceProbe(with: state)
            XCTFail("expected timeout")
        } catch let error as CloudStorageError {
            XCTAssertEqual(
                error,
                CloudStorageError.NotAvailable("iCloud namespace lookup timed out")
            )
        } catch {
            XCTFail("unexpected error: \(error)")
        }

        XCTAssertEqual(state.inspectionCount, 3)
        XCTAssertEqual(state.delays, [1, 2, 1])
        XCTAssertEqual(state.elapsed, SilentNamespaceRecoveryProbe.maximumDuration)
    }

    func testCancellableDispatchOperationSkipsWorkCancelledBeforeItStarts() async {
        let gate = QueuedCancellableDispatchOperationTestGate()
        let queue = DispatchQueue(label: "cove.tests.queued-cancellable-cloud-operation")
        gate.block(queue: queue)
        await gate.waitUntilBlocked()

        let task = Task {
            try await CancellableDispatchOperation<[String]>.run(on: queue) {
                gate.markOperationStarted()
                return ["unexpected namespace"]
            }
        }

        await Task.yield()
        task.cancel()

        do {
            _ = try await task.value
            XCTFail("expected cancellation")
        } catch is CancellationError {
            // expected
        } catch {
            XCTFail("unexpected error: \(error)")
        }

        gate.release()
        await gate.waitUntilQueueDrains(queue: queue)
        XCTAssertFalse(gate.operationStarted)
    }

    func testSilentCloudRecoveryDeadlineIncludesBlockedContainerLookup() async {
        let gate = CancellableDispatchOperationTestGate()
        let queue = DispatchQueue(label: "cove.tests.cloud-recovery-deadline")

        do {
            _ = try await SilentCloudRecoveryDeadline.run(
                watchdog: {
                    await gate.waitUntilStarted()
                },
                operation: {
                    try await CancellableDispatchOperation<[String]>.run(on: queue) {
                        gate.waitUntilReleased()
                        return ["late namespace"]
                    }
                }
            )
            XCTFail("expected timeout")
        } catch let error as CloudStorageError {
            XCTAssertEqual(
                error,
                CloudStorageError.NotAvailable("iCloud namespace lookup timed out")
            )
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
    private var inspectionDurations: [TimeInterval]
    private var recordedDelays: [TimeInterval] = []
    private var recordedMetadataTimeouts: [TimeInterval] = []

    init(
        results: [[String]],
        inspectionDurations: [TimeInterval] = []
    ) {
        self.results = results
        self.inspectionDurations = inspectionDurations
    }

    var now: TimeInterval {
        lock.withLock { currentTime }
    }

    var elapsed: TimeInterval {
        lock.withLock { currentTime }
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

        return lock.withLock {
            recordedMetadataTimeouts.append(metadataTimeout)

            if !inspectionDurations.isEmpty {
                currentTime += inspectionDurations.removeFirst()
            }

            guard !results.isEmpty else { return [] }
            return results.removeFirst()
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

    func waitUntilStarted() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                self.started.wait()
                continuation.resume()
            }
        }
    }

    func waitUntilFinished() async -> Bool {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                continuation.resume(returning: self.finished.wait(timeout: .now() + 1) == .success)
            }
        }
    }
}

private final class QueuedCancellableDispatchOperationTestGate: @unchecked Sendable {
    private let blocked = DispatchSemaphore(value: 0)
    private let releaseBlock = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var didStartOperation = false

    var operationStarted: Bool {
        lock.withLock { didStartOperation }
    }

    func block(queue: DispatchQueue) {
        queue.async {
            self.blocked.signal()
            self.releaseBlock.wait()
        }
    }

    func waitUntilBlocked() async {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                self.blocked.wait()
                continuation.resume()
            }
        }
    }

    func markOperationStarted() {
        lock.withLock {
            didStartOperation = true
        }
    }

    func release() {
        releaseBlock.signal()
    }

    func waitUntilQueueDrains(queue: DispatchQueue) async {
        await withCheckedContinuation { continuation in
            queue.async {
                continuation.resume()
            }
        }
    }
}
