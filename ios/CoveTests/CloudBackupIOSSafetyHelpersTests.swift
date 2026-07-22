@testable import Cove
import CoveCore
import XCTest

final class CloudBackupIOSSafetyHelpersTests: XCTestCase {
    func testPendingEnableRecoveryPresentationSeparatesSafeCleanupFromSupportOnly() {
        XCTAssertTrue(cloudBackupPendingEnableCleanupIsAvailable(.available))
        XCTAssertFalse(cloudBackupPendingEnableCleanupIsAvailable(.supportOnly))
        XCTAssertFalse(cloudBackupPendingEnableCleanupIsAvailable(.cleaning))
    }

    func testPendingEnableRecoverySupportEmailContainsOnlySafeContext() throws {
        let url = try XCTUnwrap(cloudBackupPendingEnableSupportEmailURL(
            supportCode: "CB-PE-004",
            appVersion: "1.3.0"
        ))
        let decoded = try XCTUnwrap(url.absoluteString.removingPercentEncoding)

        XCTAssertTrue(decoded.contains("CB-PE-004"))
        XCTAssertTrue(decoded.contains("Platform: iOS"))
        XCTAssertTrue(decoded.contains("App version: 1.3.0"))
        XCTAssertFalse(decoded.contains("namespace"))
        XCTAssertFalse(decoded.contains("credential"))
        XCTAssertFalse(decoded.contains("account"))
    }

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

    @MainActor
    func testMetadataIndexStartsOneQueryForConcurrentConsumers() async throws {
        let source = MetadataQuerySourceSpy()
        let index = ICloudMetadataIndex(source: source)
        let record = metadataRecord(name: "master-key.json", parentPath: "/cloud/namespace")
        let first = Task { try await index.currentOrInitialRecords(timeout: 1) }
        let second = Task { try await index.currentOrInitialRecords(timeout: 1) }

        await Task.yield()
        XCTAssertEqual(source.startCount, 1)

        source.send(.finishedGathering([record]))

        let firstRecords = try await first.value
        let secondRecords = try await second.value

        XCTAssertEqual(firstRecords, [record])
        XCTAssertEqual(secondRecords, [record])
        XCTAssertEqual(source.startCount, 1)
    }

    @MainActor
    func testMetadataIndexWaitsForAnItemPublishedByLaterUpdate() async throws {
        let source = MetadataQuerySourceSpy()
        let index = ICloudMetadataIndex(source: source)
        let record = metadataRecord(name: "wallet.json", parentPath: "/cloud/namespace")
        let request = Task {
            try await index.waitForItem(
                named: record.name,
                parentPath: "/cloud/namespace",
                timeout: 1
            )
        }

        await Task.yield()
        source.send(.finishedGathering([]))
        source.send(.updated([record]))

        let matchedRecord = try await request.value

        XCTAssertEqual(matchedRecord, record)
    }

    @MainActor
    func testMetadataIndexRetriesAfterQueryStartFailure() async throws {
        let source = MetadataQuerySourceSpy(startResults: [false, true])
        let index = ICloudMetadataIndex(source: source)

        do {
            _ = try await index.currentOrInitialRecords(timeout: 1)
            XCTFail("expected first query startup to fail")
        } catch let error as ICloudMetadataIndexError {
            XCTAssertEqual(error, .startFailed)
        }

        let record = metadataRecord(name: "master-key.json", parentPath: "/cloud/namespace")
        let retry = Task { try await index.currentOrInitialRecords(timeout: 1) }

        await Task.yield()
        XCTAssertEqual(source.startCount, 2)
        source.send(.finishedGathering([record]))

        let records = try await retry.value

        XCTAssertEqual(records, [record])
    }

    @MainActor
    func testBackupReadWaitsForLateMetadataAcrossAllLocations() async throws {
        let fixture = makeICloudMetadataFixture()
        defer { fixture.removeContainer() }

        let locations = backupLocations()
        let legacyURL = try fixture.helper.backupFileReadURL(
            namespace: testNamespace,
            location: locations[1]
        )
        let request = Task {
            try await fixture.helper.existingBackupFileReadURL(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: locations
            )
        }

        await Task.yield()
        fixture.source.send(.finishedGathering([]))
        fixture.source.send(.updated([
            metadataRecord(
                name: legacyURL.lastPathComponent,
                parentPath: legacyURL.deletingLastPathComponent().path
            ),
        ]))

        let resolvedURL = try await request.value

        XCTAssertEqual(resolvedURL, legacyURL)
        XCTAssertEqual(fixture.source.startCount, 1)
    }

    @MainActor
    func testBackupReadPrefersCurrentLocationWithinMetadataSnapshot() async throws {
        let fixture = makeICloudMetadataFixture()
        defer { fixture.removeContainer() }

        let locations = backupLocations()
        let currentURL = try fixture.helper.backupFileReadURL(
            namespace: testNamespace,
            location: locations[0]
        )
        let legacyURL = try fixture.helper.backupFileReadURL(
            namespace: testNamespace,
            location: locations[1]
        )
        let request = Task {
            try await fixture.helper.existingBackupFileReadURL(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: locations
            )
        }

        await Task.yield()
        fixture.source.send(.finishedGathering([
            metadataRecord(
                name: legacyURL.lastPathComponent,
                parentPath: legacyURL.deletingLastPathComponent().path
            ),
            metadataRecord(
                name: currentURL.lastPathComponent,
                parentPath: currentURL.deletingLastPathComponent().path
            ),
        ]))

        let resolvedURL = try await request.value

        XCTAssertEqual(resolvedURL, currentURL)
    }

    @MainActor
    func testBackupDeletionWaitsForLateMetadataAndCleansVisibleDuplicates() async throws {
        let fixture = makeICloudMetadataFixture()
        defer { fixture.removeContainer() }

        let locations = backupLocations()
        let currentURL = try fixture.helper.backupFileReadURL(
            namespace: testNamespace,
            location: locations[0]
        )
        let legacyURL = try fixture.helper.backupFileReadURL(
            namespace: testNamespace,
            location: locations[1]
        )
        let request = Task {
            try await fixture.helper.deleteExistingBackupFile(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: locations
            )
        }

        await Task.yield()
        fixture.source.send(.finishedGathering([]))
        try writeTestBackup(at: currentURL)
        try writeTestBackup(at: legacyURL)
        fixture.source.send(.updated([
            metadataRecord(
                name: currentURL.lastPathComponent,
                parentPath: currentURL.deletingLastPathComponent().path
            ),
        ]))
        fixture.source.send(.updated([
            metadataRecord(
                name: legacyURL.lastPathComponent,
                parentPath: legacyURL.deletingLastPathComponent().path
            ),
            metadataRecord(
                name: currentURL.lastPathComponent,
                parentPath: currentURL.deletingLastPathComponent().path
            ),
        ]))

        try await request.value

        XCTAssertFalse(FileManager.default.fileExists(atPath: currentURL.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: legacyURL.path))
    }

    @MainActor
    func testBackupReadMapsMetadataStartupAndTimeoutFailures() async throws {
        let startupFixture = makeICloudMetadataFixture(startResults: [false])
        defer { startupFixture.removeContainer() }

        do {
            _ = try await startupFixture.helper.existingBackupFileReadURL(
                namespace: testNamespace,
                recordId: "wallet-record",
                locations: backupLocations()
            )
            XCTFail("expected metadata startup failure")
        } catch CloudStorageError.NotAvailable {
        } catch {
            XCTFail("expected NotAvailable, got \(error)")
        }

        let timeoutFixture = makeICloudMetadataFixture(defaultTimeout: 0.01)
        defer { timeoutFixture.removeContainer() }
        let request = Task {
            try await timeoutFixture.helper.existingBackupFileReadURL(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: self.backupLocations()
            )
        }

        await Task.yield()
        timeoutFixture.source.send(.finishedGathering([]))

        do {
            _ = try await request.value
            XCTFail("expected metadata timeout")
        } catch CloudStorageError.Offline {
        } catch {
            XCTFail("expected Offline, got \(error)")
        }
    }

    @MainActor
    func testBackupReadCancellationDoesNotWaitForMetadataTimeout() async throws {
        let fixture = makeICloudMetadataFixture(defaultTimeout: 1)
        defer { fixture.removeContainer() }

        let request = Task {
            try await fixture.helper.existingBackupFileReadURL(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: self.backupLocations()
            )
        }

        await Task.yield()
        fixture.source.send(.finishedGathering([]))
        let startedAt = Date()
        request.cancel()

        do {
            _ = try await request.value
            XCTFail("expected cancellation")
        } catch is CancellationError {
        } catch {
            XCTFail("expected CancellationError, got \(error)")
        }

        XCTAssertLessThan(Date().timeIntervalSince(startedAt), 0.5)
    }

    func testEventuallyConsistentListingMergesLocalAndMetadataViews() throws {
        let names = try ICloudEventuallyConsistentListing.merged(
            local: ["old-1password"],
            metadata: .success(["new-apple-passwords"])
        )

        XCTAssertEqual(names, ["new-apple-passwords", "old-1password"])
    }

    func testEventuallyConsistentListingDeduplicatesAndSortsViews() throws {
        let names = try ICloudEventuallyConsistentListing.merged(
            local: ["beta", "alpha", "beta"],
            metadata: .success(["gamma", "alpha"])
        )

        XCTAssertEqual(names, ["alpha", "beta", "gamma"])
    }

    func testEventuallyConsistentListingRequiresMetadataForCompleteInventory() {
        XCTAssertThrowsError(
            try ICloudEventuallyConsistentListing.merged(
                local: ["local-only"],
                metadata: .failure(CloudStorageError.Offline("metadata unavailable"))
            )
        )
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

    private func metadataRecord(name: String, parentPath: String) -> ICloudMetadataRecord {
        let url = URL(fileURLWithPath: parentPath).appendingPathComponent(name)
        return ICloudMetadataRecord(
            name: name,
            url: url,
            resolvedPath: url.resolvingSymlinksInPath().path
        )
    }

    private var testNamespace: String {
        "0123456789abcdef0123456789abcdef"
    }

    private func backupLocations() -> [RemoteBackupLocation] {
        [
            RemoteBackupLocation(relativePath: "wallets/wallet-record.json"),
            RemoteBackupLocation(relativePath: "wallet-record.json"),
        ]
    }

    @MainActor
    private func makeICloudMetadataFixture(
        startResults: [Bool] = [true],
        defaultTimeout: TimeInterval = 1
    ) -> ICloudMetadataFixture {
        let containerURL = FileManager.default.temporaryDirectory.appendingPathComponent(
            "icloud-metadata-tests-\(UUID().uuidString)",
            isDirectory: true
        )
        let source = MetadataQuerySourceSpy(startResults: startResults)
        let index = ICloudMetadataIndex(source: source)
        let helper = ICloudDriveHelper(
            containerURLProvider: { containerURL },
            metadataIndexProvider: { index },
            defaultTimeout: defaultTimeout
        )
        return ICloudMetadataFixture(
            containerURL: containerURL,
            source: source,
            helper: helper
        )
    }

    private func writeTestBackup(at url: URL) throws {
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try Data("backup".utf8).write(to: url)
    }
}

@MainActor
private struct ICloudMetadataFixture {
    let containerURL: URL
    let source: MetadataQuerySourceSpy
    let helper: ICloudDriveHelper

    func removeContainer() {
        try? FileManager.default.removeItem(at: containerURL)
    }
}

@MainActor
private final class MetadataQuerySourceSpy: ICloudMetadataQuerySource {
    private let startResults: [Bool]
    private var onEvent: (@MainActor (ICloudMetadataQueryEvent) -> Void)?
    private(set) var startCount = 0

    init(startResults: [Bool] = [true]) {
        precondition(!startResults.isEmpty)
        self.startResults = startResults
    }

    func start(onEvent: @escaping @MainActor (ICloudMetadataQueryEvent) -> Void) -> Bool {
        let result = startResults[min(startCount, startResults.count - 1)]
        startCount += 1
        guard result else { return false }

        self.onEvent = onEvent
        return true
    }

    func send(_ event: ICloudMetadataQueryEvent) {
        onEvent?(event)
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
