@testable import Cove
import CoveCore
import SwiftUI
import XCTest

final class CloudBackupIOSSafetyHelpersTests: XCTestCase {
    func testStartupRecoveryClassifiesRecoveryRequiredWithSafeContinuationCopy() {
        let failure = classifyBootstrapFailure(
            AppInitError.RecoveryRequired(message: "wallet path and cleanup details")
        )

        XCTAssertEqual(failure, .recoveryRequired)
        XCTAssertTrue(startupRecoveryContinuationMessage.contains("Continue"))
        XCTAssertFalse(startupRecoveryContinuationMessage.contains("wallet path"))
    }

    func testStartupRecoveryOffersRetryWhenKeychainIsUnreadable() {
        let error = AppInitError.KeychainUnavailable(message: "status=-25308")
        let failure = classifyBootstrapFailure(error)

        XCTAssertEqual(failure, .keychainUnavailable(error.localizedDescription))
        XCTAssertNotEqual(failure, .catastrophicRecovery)
        XCTAssertTrue(startupKeychainUnavailableMessage.contains("Try Again"))
    }

    func testStartupRecoveryLeavesOtherFatalErrorsWithoutRetryState() {
        let error = AppInitError.DatabaseVerificationFailed(message: "verification failed")
        let failure = classifyBootstrapFailure(error)

        XCTAssertEqual(failure, .fatal(error.localizedDescription))
    }

    /// Recovery offers Remove Incomplete Setup only while Rust offers the cleanup, so a support-only
    /// setup shows its support code with no way to remove what Cove can't safely remove
    func testPendingEnableRecoveryOffersCleanupOnlyWhenRustDoes() async throws {
        let fixture = try XCTUnwrap(cloudBackupScreenFixtures().first { $0.name.hasSuffix(":pending-enable-recovery") })
        guard case let .pendingEnableRecovery(recovery) = fixture.screen.body else {
            return XCTFail("the pending-enable-recovery fixture has a \(fixture.screen.body) body")
        }

        let offered = try await render(CloudBackupPendingEnableRecoveryView(
            recovery: recovery,
            actions: fixture.screen.actions,
            onRemoveIncompleteSetup: {},
            onCancel: {}
        ))
        XCTAssertEqual(offered.actions[.confirmPendingEnableCleanup], true, "cleanup Rust offers has no enabled control")

        var supportOnly = recovery
        supportOnly.cleanup = .supportOnly
        let withoutCleanup = CloudBackupActions(
            entries: fixture.screen.actions.entries.filter { $0.action != .confirmPendingEnableCleanup }
        )

        let notOffered = try await render(CloudBackupPendingEnableRecoveryView(
            recovery: supportOnly,
            actions: withoutCleanup,
            onRemoveIncompleteSetup: {},
            onCancel: {}
        ))
        XCTAssertNil(notOffered.actions[.confirmPendingEnableCleanup], "a support-only setup drew a cleanup control")
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
            cloudBackupPendingUploadAccessibilityStatus(syncState: .blocked("iCloud authorization required")),
            .authorizationRequired
        )
        XCTAssertEqual(
            cloudBackupPendingUploadAccessibilityStatus(syncState: .failed("upload confirmation failed")),
            .failed
        )
        XCTAssertEqual(cloudBackupPendingUploadAccessibilityStatus(syncState: .syncing), .confirming)
        XCTAssertEqual(cloudBackupPendingUploadAccessibilityStatus(syncState: nil), .confirming)
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

    /// A wallet whose secret left this device says so, so its row doesn't read as fully backed up
    func testSecretMissingLocallyRowSaysTheCloudCopyRestoresIt() throws {
        let fixture = try XCTUnwrap(
            cloudBackupScreenFixtures().first { $0.name.hasSuffix(":configured-secret-missing-locally") }
        )
        guard case let .configured(configured) = fixture.screen.body else {
            return XCTFail("the fixture is configured")
        }
        let rows = configured.sections.flatMap { section -> [CloudBackupOwnRow] in
            guard case let .wallets(wallets) = section else { return [] }
            return wallets.upToDate + wallets.needsSync
        }
        let row = try XCTUnwrap(rows.first)
        XCTAssertEqual(row.record, .secretMissingLocally)

        let label = cloudBackupWalletAccessibilityLabel(item: row.wallet, record: row.record)
        XCTAssertTrue(label.contains(cloudBackupSecretMissingLocallyMessage), label)
        XCTAssertFalse(
            cloudBackupWalletAccessibilityLabel(item: row.wallet, record: .current)
                .contains(cloudBackupSecretMissingLocallyMessage)
        )
    }

    /// Restore All titles carry Rust's wallet count whether or not the action can run now, since
    /// availability comes from the screen's actions and draws as a disabled control with its reason
    func testRestoreAllPresentationTitlesCarryRustsCount() {
        XCTAssertEqual(cloudBackupRestoreAllPresentation(state: .notShown), .hidden)

        for state in [CloudBackupRestoreAllState.startAvailable(walletCount: 3), .startDisabled(walletCount: 3)] {
            XCTAssertEqual(cloudBackupRestoreAllPresentation(state: state), .start(title: "Restore All (3)"))
        }

        for state in [CloudBackupRestoreAllState.retryAvailable(walletCount: 1), .retryDisabled(walletCount: 1)] {
            XCTAssertEqual(cloudBackupRestoreAllPresentation(state: state), .retry(title: "Retry Remaining (1)"))
        }
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
                cancellationRequested: false
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

        XCTAssertTrue(progress.cancellationRequested)
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

        await source.waitUntilStarted()
        XCTAssertEqual(source.startCount, 1)

        source.send(.finishedGathering([record]))

        let firstRecords = try await first.value
        let secondRecords = try await second.value

        XCTAssertEqual(firstRecords, [record])
        XCTAssertEqual(secondRecords, [record])
        XCTAssertEqual(source.startCount, 1)
    }

    /// What was queued before the manager read its snapshot, a Turn Off finishing, then a new
    /// Enable and its completion, is skipped by revision instead of replayed over the snapshot:
    /// the screen never goes back to Finishing or Disabled, and the completion event, which has
    /// no revision, still arrives in order. The next delta after the snapshot applies
    func testScreenDeltasQueuedBeforeTheSnapshotAreSkipped() throws {
        let fixtures = cloudBackupScreenFixtures()
        let screen = { (name: String) in
            try XCTUnwrap(fixtures.first { $0.name.hasSuffix(":\(name)") }?.screen)
        }
        let configured = try screen("configured")
        let finishing = try screen("finishing-disable")
        let disabled = try screen("disabled")
        let delta = { (revision: UInt64, body: CloudBackupScreenBody) in
            CloudBackupScreenDelta(
                revision: revision,
                body: body,
                settingsRow: nil,
                healNotice: nil,
                lastOutcome: nil,
                rootPrompt: nil,
                actions: nil
            )
        }
        let snapshot = CloudBackupPublishedScreen(revision: 3, screen: configured)

        // the fake handle is never called: the manager is not listening, and nothing dispatches
        let manager = CloudBackupManager(rust: RustCloudBackupManager(noHandle: .init()), published: snapshot)

        manager.apply(.screen(delta(1, finishing.body)))
        XCTAssertEqual(manager.published, snapshot)

        manager.apply(.screen(delta(2, disabled.body)))
        XCTAssertEqual(manager.published, snapshot)

        manager.apply(.enableCompleted(CloudBackupEnableContext(
            savedPasskeyConfirmation: .manual,
            verificationSource: .settings
        )))
        XCTAssertNotNil(manager.enableCompletion)

        manager.apply(.screen(delta(3, configured.body)))
        XCTAssertEqual(manager.published, snapshot)
        XCTAssertNotNil(manager.enableCompletion)

        manager.apply(.screen(delta(4, disabled.body)))
        XCTAssertEqual(manager.published.revision, 4)
        XCTAssertEqual(manager.screen.body, .disabled)
        XCTAssertNil(manager.enableCompletion)
    }
}

extension CloudBackupIOSSafetyHelpersTests {
    @MainActor
    func testMetadataIndexDoesNotSettleBeforeInitialGatheringFinishes() async throws {
        let source = MetadataQuerySourceSpy()
        let settleSleep = MetadataSettleSleepSpy(blockedCalls: [1])
        let index = ICloudMetadataIndex(
            source: source,
            settleSleep: { duration in try await settleSleep.sleep(for: duration) }
        )
        let record = metadataRecord(name: "master-key.json", parentPath: "/cloud/namespace")
        let request = Task {
            try await index.settledRecords(timeout: 1, settleInterval: 0.5)
        }

        await source.waitUntilStarted()
        XCTAssertEqual(settleSleep.durations, [])

        source.send(.finishedGathering([record]))
        await settleSleep.waitUntilCalled(count: 1)
        settleSleep.resume(call: 1)

        let records = try await request.value
        XCTAssertEqual(records, [record])
        XCTAssertEqual(settleSleep.durations, [0.5])
    }

    @MainActor
    func testMetadataIndexReusesStrongestSettledGeneration() async throws {
        let source = MetadataQuerySourceSpy()
        let settleSleep = MetadataSettleSleepSpy(blockedCalls: [1])
        let index = ICloudMetadataIndex(
            source: source,
            settleSleep: { duration in try await settleSleep.sleep(for: duration) }
        )
        let record = metadataRecord(name: "master-key.json", parentPath: "/cloud/namespace")
        let initial = Task {
            try await index.settledRecords(timeout: 1, settleInterval: 0.5)
        }

        await source.waitUntilStarted()
        source.send(.finishedGathering([record]))
        await settleSleep.waitUntilCalled(count: 1)
        settleSleep.resume(call: 1)
        _ = try await initial.value

        let weakerRecords = try await index.settledRecords(timeout: 1, settleInterval: 0.25)
        let equalRecords = try await index.settledRecords(timeout: 1, settleInterval: 0.5)

        XCTAssertEqual(weakerRecords, [record])
        XCTAssertEqual(equalRecords, [record])
        XCTAssertEqual(settleSleep.durations, [0.5])

        let strongerRecords = try await index.settledRecords(timeout: 1, settleInterval: 0.75)

        XCTAssertEqual(strongerRecords, [record])
        XCTAssertEqual(settleSleep.durations, [0.5, 0.75])
    }

    @MainActor
    func testMetadataIndexInvalidatesSettledGenerationOnUpdates() async throws {
        let source = MetadataQuerySourceSpy()
        let settleSleep = MetadataSettleSleepSpy(blockedCalls: [1, 2])
        let index = ICloudMetadataIndex(
            source: source,
            settleSleep: { duration in try await settleSleep.sleep(for: duration) }
        )
        let initialRecord = metadataRecord(
            name: "master-key.json",
            parentPath: "/cloud/namespace"
        )
        let firstUpdate = metadataRecord(name: "wallet-1.json", parentPath: "/cloud/namespace")
        let secondUpdate = metadataRecord(name: "wallet-2.json", parentPath: "/cloud/namespace")
        let initial = Task {
            try await index.settledRecords(timeout: 1, settleInterval: 0.5)
        }

        await source.waitUntilStarted()
        source.send(.finishedGathering([initialRecord]))
        await settleSleep.waitUntilCalled(count: 1)
        settleSleep.resume(call: 1)
        _ = try await initial.value

        source.send(.updated([firstUpdate]))
        let refreshed = Task {
            try await index.settledRecords(timeout: 1, settleInterval: 0.5)
        }
        await settleSleep.waitUntilCalled(count: 2)

        source.send(.updated([secondUpdate]))
        settleSleep.resume(call: 2)
        await settleSleep.waitUntilCalled(count: 3)

        let records = try await refreshed.value
        XCTAssertEqual(records, [secondUpdate])
        XCTAssertEqual(settleSleep.durations, [0.5, 0.5, 0.5])
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

        await source.waitUntilStarted()
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

        await source.waitUntilStarted(count: 2)
        XCTAssertEqual(source.startCount, 2)
        source.send(.finishedGathering([record]))

        let records = try await retry.value

        XCTAssertEqual(records, [record])
    }

    @MainActor
    func testBackupReadUsesLocalTargetBeforeMetadata() async throws {
        // a slow simulator can exceed a short deadline on the direct read and fall back to metadata
        let fixture = makeICloudMetadataFixture(defaultTimeout: 60)
        defer { fixture.removeContainer() }

        let location = try XCTUnwrap(backupLocations().first)
        let localURL = try fixture.helper.backupFileReadURL(
            namespace: testNamespace,
            location: location
        )
        try writeTestBackup(at: localURL)

        let target = try await fixture.helper.existingBackupFileReadTarget(
            namespace: testNamespace,
            recordId: "wallet-record",
            locations: backupLocations()
        )
        let data = try await fixture.helper.downloadFile(
            target: target,
            recordId: "wallet-record"
        )

        XCTAssertEqual(target, .local(localURL))
        XCTAssertEqual(data, Data("backup".utf8))
        XCTAssertEqual(fixture.source.startCount, 0)
    }

    func testICloudReadAttemptDeadlineReturnsWithoutWaitingForSlowRead() async throws {
        let startedAt = Date()

        do {
            _ = try await ICloudReadAttemptDeadline.run(timeout: 0.01) {
                try await Task.sleep(for: .seconds(10))
                return Data()
            }
            XCTFail("expected read timeout")
        } catch ICloudReadAttemptDeadlineError.timedOut {
        } catch {
            XCTFail("expected read timeout, got \(error)")
        }

        XCTAssertLessThan(Date().timeIntervalSince(startedAt), 0.5)
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
            try await fixture.helper.existingBackupFileReadTarget(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: locations,
                lookupMode: .waitForSync
            )
        }

        await fixture.source.waitUntilStarted()
        fixture.source.send(.finishedGathering([]))
        fixture.source.send(.updated([
            metadataRecord(
                name: legacyURL.lastPathComponent,
                parentPath: legacyURL.deletingLastPathComponent().path
            ),
        ]))

        let target = try await request.value

        XCTAssertEqual(
            target,
            .provider(
                legacyURL,
                metadataPath: legacyURL.resolvingSymlinksInPath().path
            )
        )
        XCTAssertEqual(fixture.source.startCount, 1)
    }

    @MainActor
    func testSilentBackupReadReturnsNotFoundFromCompletedSnapshot() async throws {
        let fixture = makeICloudMetadataFixture(defaultTimeout: 60)
        defer { fixture.removeContainer() }
        let request = Task {
            try await fixture.helper.existingBackupFileReadTarget(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: self.backupLocations(),
                lookupMode: .currentSnapshot
            )
        }

        await fixture.source.waitUntilStarted()
        fixture.source.send(.finishedGathering([]))

        do {
            _ = try await request.value
            XCTFail("expected missing snapshot item")
        } catch CloudStorageError.NotFound {
        } catch {
            XCTFail("expected NotFound, got \(error)")
        }

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
            try await fixture.helper.existingBackupFileReadTarget(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: locations
            )
        }

        await fixture.source.waitUntilStarted()
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

        let target = try await request.value

        XCTAssertEqual(
            target,
            .provider(
                currentURL,
                metadataPath: currentURL.resolvingSymlinksInPath().path
            )
        )
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

        await fixture.source.waitUntilStarted()
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
}

extension CloudBackupIOSSafetyHelpersTests {
    @MainActor
    func testBackupReadMapsMetadataStartupAndTimeoutFailures() async throws {
        let startupFixture = makeICloudMetadataFixture(startResults: [false])
        defer { startupFixture.removeContainer() }

        do {
            _ = try await startupFixture.helper.existingBackupFileReadTarget(
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
            try await timeoutFixture.helper.existingBackupFileReadTarget(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: self.backupLocations()
            )
        }

        await timeoutFixture.source.waitUntilStarted()
        timeoutFixture.source.send(.finishedGathering([]))

        do {
            _ = try await request.value
            XCTFail("expected metadata timeout")
        } catch CloudStorageError.SyncPending {
        } catch {
            XCTFail("expected SyncPending, got \(error)")
        }
    }

    @MainActor
    func testBackupUploadStatusReturnsNotFoundAfterSuccessfulMetadataLookup() async throws {
        let fixture = makeICloudMetadataFixture()
        defer { fixture.removeContainer() }
        let request = Task {
            try await fixture.helper.isBackupUploaded(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: self.backupLocations()
            )
        }

        await fixture.source.waitUntilStarted()
        fixture.source.send(.finishedGathering([]))

        let status = try await request.value

        XCTAssertEqual(status, .notFound)
    }

    @MainActor
    func testBackupUploadStatusPropagatesMetadataStartupFailure() async throws {
        let fixture = makeICloudMetadataFixture(startResults: [false])
        defer { fixture.removeContainer() }

        do {
            _ = try await fixture.helper.isBackupUploaded(
                namespace: testNamespace,
                recordId: "wallet-record",
                locations: backupLocations()
            )
            XCTFail("expected metadata startup failure")
        } catch CloudStorageError.NotAvailable {
        } catch {
            XCTFail("expected NotAvailable, got \(error)")
        }
    }

    @MainActor
    func testBackupUploadStatusPropagatesMetadataTimeout() async throws {
        let fixture = makeICloudMetadataFixture(metadataListingTimeout: 0.01)
        defer { fixture.removeContainer() }

        do {
            _ = try await fixture.helper.isBackupUploaded(
                namespace: testNamespace,
                recordId: "wallet-record",
                locations: backupLocations()
            )
            XCTFail("expected metadata timeout")
        } catch CloudStorageError.SyncPending {
        } catch {
            XCTFail("expected SyncPending, got \(error)")
        }
    }

    @MainActor
    func testBackupReadCancellationDoesNotWaitForMetadataTimeout() async throws {
        let fixture = makeICloudMetadataFixture(defaultTimeout: 1)
        defer { fixture.removeContainer() }

        let request = Task {
            try await fixture.helper.existingBackupFileReadTarget(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: self.backupLocations()
            )
        }

        await fixture.source.waitUntilStarted()
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
        settleSleep: @escaping ICloudMetadataSettleSleep = { duration in
            try await Task.sleep(for: .seconds(duration))
        },
        coordinatedDeleter: @escaping @Sendable (URL, String) throws -> URL = { url, missingItemID in
            try ICloudDriveHelper.coordinatedDelete(at: url, missingItemID: missingItemID)
        },
        defaultTimeout: TimeInterval = 1,
        metadataListingTimeout: TimeInterval = 5
    ) -> ICloudMetadataFixture {
        let containerURL = FileManager.default.temporaryDirectory.appendingPathComponent(
            "icloud-metadata-tests-\(UUID().uuidString)",
            isDirectory: true
        )
        let source = MetadataQuerySourceSpy(startResults: startResults)
        let index = ICloudMetadataIndex(
            source: source,
            settleSleep: settleSleep
        )
        let helper = ICloudDriveHelper(
            containerURLProvider: { containerURL },
            metadataIndexProvider: { index },
            coordinatedDeleter: coordinatedDeleter,
            defaultTimeout: defaultTimeout,
            metadataListingTimeout: metadataListingTimeout
        )
        return ICloudMetadataFixture(
            containerURL: containerURL,
            source: source,
            index: index,
            helper: helper
        )
    }

    /// Completes the index's initial gather with `records`
    @MainActor
    private func seedFinishedGathering(
        _ fixture: ICloudMetadataFixture,
        _ records: [ICloudMetadataRecord]
    ) async throws {
        let initial = Task { try await fixture.index.currentOrInitialRecords(timeout: 1) }
        await fixture.source.waitUntilStarted()
        fixture.source.send(.finishedGathering(records))
        _ = try await initial.value
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
extension CloudBackupIOSSafetyHelpersTests {
    func testMetadataDeletionImmediatelyFiltersReadersAndStaleUpdates() async throws {
        let settleSleep = MetadataSettleSleepSpy(blockedCalls: [])
        let fixture = makeICloudMetadataFixture(
            settleSleep: { duration in try await settleSleep.sleep(for: duration) }
        )
        defer { fixture.removeContainer() }

        let deleted = metadataRecord(name: "wallet.json", parentPath: "/cloud/namespace")
        let unrelated = metadataRecord(name: "other.json", parentPath: "/cloud/namespace")
        let initial = Task {
            try await fixture.index.settledRecords(timeout: 1, settleInterval: 0.5)
        }

        await fixture.source.waitUntilStarted()
        fixture.source.send(.finishedGathering([deleted, unrelated]))
        let initialRecords = try await initial.value
        XCTAssertEqual(initialRecords, [deleted, unrelated])

        var observerCount = 0
        let observerID = fixture.index.addObserver { observerCount += 1 }
        fixture.index.markDeleted(resolvedPaths: [deleted.resolvedPath])

        let currentRecords = try await fixture.index.currentOrInitialRecords(timeout: 1)
        let deletedItem = try await fixture.index.itemIfPresent(
            named: deleted.name,
            parentPath: "/cloud/namespace",
            timeout: 1
        )
        XCTAssertEqual(currentRecords, [unrelated])
        XCTAssertNil(deletedItem)
        XCTAssertEqual(
            fixture.index.visibleItems(matching: [
                ICloudMetadataCandidate(name: unrelated.name, parentPath: "/cloud/namespace"),
            ]),
            [unrelated]
        )
        let settledRecords = try await fixture.index.settledRecords(
            timeout: 1,
            settleInterval: 0.5
        )
        XCTAssertEqual(settledRecords, [unrelated])
        XCTAssertEqual(settleSleep.durations, [0.5])
        XCTAssertEqual(observerCount, 0)

        fixture.source.send(.updated([deleted, unrelated]))
        fixture.source.send(.updated([deleted, unrelated]))

        let staleUpdateRecords = try await fixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(staleUpdateRecords, [unrelated])
        XCTAssertEqual(
            fixture.index.visibleItems(matching: [
                ICloudMetadataCandidate(name: deleted.name, parentPath: "/cloud/namespace"),
            ]),
            []
        )
        XCTAssertEqual(observerCount, 2)
        XCTAssertEqual(fixture.source.startCount, 1)

        let refreshedRecords = try await fixture.index.settledRecords(
            timeout: 1,
            settleInterval: 0.5
        )
        XCTAssertEqual(refreshedRecords, [unrelated])
        XCTAssertEqual(settleSleep.durations, [0.5, 0.5])
        fixture.index.removeObserver(observerID)
    }

    func testMetadataDeletionSurvivesSnapshotsThatOmitThePath() async throws {
        let record = metadataRecord(name: "wallet.json", parentPath: "/cloud/namespace")

        // an empty cold-start gather is not proof of deletion
        let initialFixture = makeICloudMetadataFixture()
        initialFixture.index.markDeleted(resolvedPaths: [record.resolvedPath])
        let initial = Task { try await initialFixture.index.currentOrInitialRecords(timeout: 1) }
        await initialFixture.source.waitUntilStarted()
        initialFixture.source.send(.finishedGathering([]))
        let absentInitialRecords = try await initial.value
        XCTAssertEqual(absentInitialRecords, [])
        initialFixture.source.send(.updated([record]))
        let staleInitialRecords = try await initialFixture.index.currentOrInitialRecords(
            timeout: 1
        )
        XCTAssertEqual(staleInitialRecords, [])

        let liveFixture = makeICloudMetadataFixture()
        try await seedFinishedGathering(liveFixture, [record])
        liveFixture.index.markDeleted(resolvedPaths: [record.resolvedPath])
        liveFixture.source.send(.updated([]))
        liveFixture.source.send(.updated([record]))
        let staleLiveRecords = try await liveFixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(staleLiveRecords, [])

        // a later upload of the same file releases the tombstone without another event
        liveFixture.index.clearDeletion(resolvedPath: record.resolvedPath)
        let reuploadedRecords = try await liveFixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(reuploadedRecords, [record])
    }

    func testMetadataDeletionCoversDescendantsWithoutMatchingTextPrefix() async throws {
        let fixture = makeICloudMetadataFixture()
        let directoryPath = "/cloud/namespace"
        let child = metadataRecord(name: "wallet.json", parentPath: directoryPath)
        let nested = metadataRecord(name: "key.json", parentPath: directoryPath + "/nested")
        let sibling = metadataRecord(name: "other.json", parentPath: directoryPath + "-other")
        try await seedFinishedGathering(fixture, [child, nested, sibling])

        fixture.index.markDeleted(resolvedPaths: [directoryPath, directoryPath])
        let records = try await fixture.index.currentOrInitialRecords(timeout: 1)

        XCTAssertEqual(records, [sibling])
        XCTAssertEqual(
            ICloudMetadataProjection.subdirectoryNames(in: records, parentPath: "/cloud"),
            ["namespace-other"]
        )
        XCTAssertEqual(
            ICloudMetadataProjection.fileNames(
                in: records,
                parentPath: directoryPath,
                prefix: "wallet"
            ),
            []
        )
    }

    func testSuccessfulBackupDeleteHidesCurrentAndLegacyMetadataImmediately() async throws {
        for location in backupLocations() {
            let fixture = makeICloudMetadataFixture(settleSleep: { _ in })
            defer { fixture.removeContainer() }

            let url = try fixture.helper.backupFileReadURL(
                namespace: testNamespace,
                location: location
            )
            try writeTestBackup(at: url)
            let record = metadataRecord(
                name: url.lastPathComponent,
                parentPath: url.deletingLastPathComponent().path
            )
            try await seedFinishedGathering(fixture, [record])

            try await fixture.helper.deleteExistingBackupFile(
                namespace: testNamespace,
                recordId: "wallet-record",
                locations: [location]
            )

            let fileNames = try await fixture.helper.metadataFileNames(
                namespacePath: url.deletingLastPathComponent().path,
                prefix: csppWalletFilePrefix()
            )
            XCTAssertEqual(fileNames, [])
            fixture.source.send(.updated([record]))
            let staleRecords = try await fixture.index.currentOrInitialRecords(timeout: 1)
            XCTAssertEqual(staleRecords, [])

            do {
                _ = try await fixture.helper.existingBackupFileReadTarget(
                    namespace: testNamespace,
                    recordId: "wallet-record",
                    locations: [location],
                    lookupMode: .currentSnapshot
                )
                XCTFail("expected deleted metadata item to be absent")
            } catch CloudStorageError.NotFound {}
        }
    }

    func testBackupDeleteRecordsCoordinatorResolvedURL() async throws {
        let location = try XCTUnwrap(backupLocations().first)
        let redirectedURL = URL(fileURLWithPath: "/provider/\(testNamespace)/wallet-record.json")
        let deleteStub = CoordinatedDeleteStub(redirectTo: redirectedURL)
        let fixture = makeICloudMetadataFixture(
            coordinatedDeleter: { url, id in
                try deleteStub.delete(url: url, missingItemID: id)
            }
        )
        defer { fixture.removeContainer() }

        let requestedURL = try fixture.helper.backupFileReadURL(
            namespace: testNamespace,
            location: location
        )
        try writeTestBackup(at: requestedURL)
        let providerRecord = ICloudMetadataRecord(
            name: requestedURL.lastPathComponent,
            url: redirectedURL,
            resolvedPath: redirectedURL.resolvingSymlinksInPath().path
        )
        try await seedFinishedGathering(fixture, [providerRecord])

        try await fixture.helper.deleteExistingBackupFile(
            namespace: testNamespace,
            recordId: "wallet-record",
            locations: [location]
        )

        XCTAssertEqual(deleteStub.deletedURLs, [requestedURL])
        var visibleRecords = try await fixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(visibleRecords, [])
        fixture.source.send(.updated([providerRecord]))
        visibleRecords = try await fixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(visibleRecords, [])
    }

    func testBackupDeleteRecordsPartialSuccessBeforeFailure() async throws {
        let deleteStub = CoordinatedDeleteStub(failOnCall: 2)
        let fixture = makeICloudMetadataFixture(
            coordinatedDeleter: { url, id in
                try deleteStub.delete(url: url, missingItemID: id)
            }
        )
        defer { fixture.removeContainer() }
        let locations = backupLocations()
        let urls = try locations.map {
            try fixture.helper.backupFileReadURL(namespace: testNamespace, location: $0)
        }
        for url in urls {
            try writeTestBackup(at: url)
        }
        let records = urls.map {
            metadataRecord(name: $0.lastPathComponent, parentPath: $0.deletingLastPathComponent().path)
        }
        try await seedFinishedGathering(fixture, records)

        do {
            try await fixture.helper.deleteExistingBackupFile(
                namespace: testNamespace,
                recordId: "wallet-record",
                locations: locations
            )
            XCTFail("expected injected delete failure")
        } catch CloudStorageError.UploadFailed {}

        let remainingRecords = try await fixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(remainingRecords, [records[1]])
    }

    func testNamespaceDeleteHidesLocalDescendants() async throws {
        let localFixture = makeICloudMetadataFixture()
        defer { localFixture.removeContainer() }
        let localURL = try localFixture.helper.namespaceDirectoryReadURL(namespace: testNamespace)
        let localChild = localURL.appendingPathComponent("wallet.json")
        try writeTestBackup(at: localChild)
        let localRecords = [
            metadataRecord(
                name: localURL.lastPathComponent,
                parentPath: localURL.deletingLastPathComponent().path
            ),
            metadataRecord(name: localChild.lastPathComponent, parentPath: localURL.path),
        ]
        try await seedFinishedGathering(localFixture, localRecords)

        try await localFixture.helper.deleteNamespaceDirectory(namespace: testNamespace)
        var visibleRecords = try await localFixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(visibleRecords, [])
        localFixture.source.send(.updated(localRecords))
        visibleRecords = try await localFixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(visibleRecords, [])
    }

    func testNamespaceDeleteHidesMetadataFallbackDescendants() async throws {
        let fallbackStub = CoordinatedDeleteStub(removeFiles: false)
        let fallbackFixture = makeICloudMetadataFixture(
            coordinatedDeleter: { url, id in
                try fallbackStub.delete(url: url, missingItemID: id)
            }
        )
        defer { fallbackFixture.removeContainer() }
        let requestedURL = try fallbackFixture.helper.namespaceDirectoryReadURL(
            namespace: testNamespace
        )
        let providerURL = URL(fileURLWithPath: "/provider/\(testNamespace)")
        let namespaceRecord = ICloudMetadataRecord(
            name: testNamespace,
            url: providerURL,
            resolvedPath: requestedURL.resolvingSymlinksInPath().path
        )
        let childRecord = metadataRecord(name: "wallet.json", parentPath: requestedURL.path)
        let fallbackInitial = Task {
            try await fallbackFixture.index.currentOrInitialRecords(timeout: 1)
        }
        await fallbackFixture.source.waitUntilStarted()
        fallbackFixture.source.send(.finishedGathering([namespaceRecord, childRecord]))
        _ = try await fallbackInitial.value

        try await fallbackFixture.helper.deleteNamespaceDirectory(namespace: testNamespace)

        XCTAssertEqual(fallbackStub.deletedURLs, [providerURL])
        var visibleRecords = try await fallbackFixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(visibleRecords, [])
        fallbackFixture.source.send(.updated([namespaceRecord, childRecord]))
        visibleRecords = try await fallbackFixture.index.currentOrInitialRecords(timeout: 1)
        XCTAssertEqual(visibleRecords, [])
    }
}

@MainActor
private struct ICloudMetadataFixture {
    let containerURL: URL
    let source: MetadataQuerySourceSpy
    let index: ICloudMetadataIndex
    let helper: ICloudDriveHelper

    func removeContainer() {
        guard FileManager.default.fileExists(atPath: containerURL.path) else { return }

        try? FileManager.default.removeItem(at: containerURL)
    }
}

private final class CoordinatedDeleteStub: @unchecked Sendable {
    private let lock = NSLock()
    private let failOnCall: Int?
    private let removeFiles: Bool
    private let redirectedURL: URL?
    private var calls = 0
    private var urls: [URL] = []

    init(
        failOnCall: Int? = nil,
        removeFiles: Bool = true,
        redirectTo redirectedURL: URL? = nil
    ) {
        self.failOnCall = failOnCall
        self.removeFiles = removeFiles
        self.redirectedURL = redirectedURL
    }

    var deletedURLs: [URL] {
        lock.withLock { urls }
    }

    func delete(url: URL, missingItemID _: String) throws -> URL {
        let call = lock.withLock {
            calls += 1
            return calls
        }
        if call == failOnCall {
            throw CloudStorageError.UploadFailed("injected delete failure")
        }

        if removeFiles {
            try FileManager.default.removeItem(at: url)
        }
        lock.withLock {
            urls.append(url)
        }

        return redirectedURL ?? url
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
        startWaiters
            .filter { $0.count <= startCount }
            .forEach { $0.expectation.fulfill() }
        startWaiters.removeAll { $0.count <= startCount }

        guard result else { return false }

        self.onEvent = onEvent
        return true
    }

    func waitUntilStarted(count: Int = 1, timeout: TimeInterval = 1) async {
        guard startCount < count else { return }

        let expectation = XCTestExpectation(
            description: "metadata query source starts \(count) time(s)"
        )
        startWaiters.append((count, expectation))

        let result = await XCTWaiter.fulfillment(of: [expectation], timeout: timeout)
        startWaiters.removeAll { $0.expectation === expectation }

        XCTAssertEqual(
            result,
            .completed,
            "metadata query source started \(startCount) of \(count) expected time(s)"
        )
    }

    func send(_ event: ICloudMetadataQueryEvent) {
        onEvent?(event)
    }

    private var startWaiters: [(count: Int, expectation: XCTestExpectation)] = []
}

@MainActor
private final class MetadataSettleSleepSpy {
    private let blockedCalls: Set<Int>
    private var continuations: [Int: CheckedContinuation<Void, Error>] = [:]
    private var callWaiters: [(count: Int, expectation: XCTestExpectation)] = []
    private(set) var durations: [TimeInterval] = []

    init(blockedCalls: Set<Int>) {
        self.blockedCalls = blockedCalls
    }

    func sleep(for duration: TimeInterval) async throws {
        try Task.checkCancellation()
        durations.append(duration)
        let call = durations.count
        callWaiters
            .filter { $0.count <= call }
            .forEach { $0.expectation.fulfill() }
        callWaiters.removeAll { $0.count <= call }

        guard blockedCalls.contains(call) else { return }

        try await withCheckedThrowingContinuation { continuation in
            continuations[call] = continuation
        }
    }

    func waitUntilCalled(count: Int, timeout: TimeInterval = 1) async {
        guard durations.count < count else { return }

        let expectation = XCTestExpectation(description: "metadata settle sleep call \(count)")
        callWaiters.append((count, expectation))

        let result = await XCTWaiter.fulfillment(of: [expectation], timeout: timeout)
        callWaiters.removeAll { $0.expectation === expectation }

        XCTAssertEqual(
            result,
            .completed,
            "metadata settle sleep reached \(durations.count) of \(count) expected call(s)"
        )
    }

    func resume(call: Int) {
        continuations.removeValue(forKey: call)?.resume()
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

/// iCloud evicts a file it has uploaded by replacing it with a hidden `.Name.icloud`
/// placeholder, so a restored or space-starved device lists placeholders where its backups are
@MainActor
extension CloudBackupIOSSafetyHelpersTests {
    func testLocalListingsReportEvictedPlaceholdersUnderTheirRealNames() throws {
        let fixture = makeICloudMetadataFixture()
        defer { fixture.removeContainer() }

        let evictedNamespace = "fedcba9876543210fedcba9876543210"
        let namespacesRoot = try fixture.helper.namespacesRootReadURL()
        let walletsDirectory = try fixture.helper.walletsDirectoryReadURL(namespace: testNamespace)
        let prefix = csppWalletFilePrefix()
        try FileManager.default.createDirectory(
            at: walletsDirectory,
            withIntermediateDirectories: true
        )

        // a downloaded namespace, an evicted one, and files that are not namespaces
        try writeEmptyFile(".\(evictedNamespace).icloud", in: namespacesRoot)
        try writeEmptyFile("notes.txt", in: namespacesRoot)
        try writeEmptyFile(".DS_Store", in: namespacesRoot)

        // a downloaded wallet, an evicted one, one present both ways, and unrelated files
        try writeEmptyFile("\(prefix)downloaded.json", in: walletsDirectory)
        try writeEmptyFile(".\(prefix)evicted.json.icloud", in: walletsDirectory)
        try writeEmptyFile("\(prefix)both.json", in: walletsDirectory)
        try writeEmptyFile(".\(prefix)both.json.icloud", in: walletsDirectory)
        try writeEmptyFile(".other.json.icloud", in: walletsDirectory)
        try writeEmptyFile("other.json", in: walletsDirectory)

        XCTAssertEqual(
            fixture.helper.locallyVisibleSubdirectoryNames(parentPath: namespacesRoot.path),
            [testNamespace, evictedNamespace].sorted()
        )

        let expectedWallets = ["both", "downloaded", "evicted"].map { "\(prefix)\($0).json" }
        XCTAssertEqual(
            fixture.helper.locallyVisibleFileNames(
                namespacePath: walletsDirectory.path,
                prefix: prefix
            ),
            expectedWallets
        )
    }

    func testBackupReadOfEvictedFileWaitsForMetadataInsteadOfReadingThePlaceholder() async throws {
        let fixture = makeICloudMetadataFixture()
        defer { fixture.removeContainer() }

        let locations = backupLocations()
        let currentURL = try fixture.helper.backupFileReadURL(
            namespace: testNamespace,
            location: locations[0]
        )
        try FileManager.default.createDirectory(
            at: currentURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try writeEmptyFile(
            ".\(currentURL.lastPathComponent).icloud",
            in: currentURL.deletingLastPathComponent()
        )

        let request = Task {
            try await fixture.helper.existingBackupFileReadTarget(
                namespace: self.testNamespace,
                recordId: "wallet-record",
                locations: locations,
                lookupMode: .waitForSync
            )
        }

        // the placeholder is not a readable copy, so the read goes to the provider's metadata
        await fixture.source.waitUntilStarted()
        fixture.source.send(.finishedGathering([]))
        fixture.source.send(.updated([
            metadataRecord(
                name: currentURL.lastPathComponent,
                parentPath: currentURL.deletingLastPathComponent().path
            ),
        ]))

        let target = try await request.value

        XCTAssertEqual(
            target,
            .provider(currentURL, metadataPath: currentURL.resolvingSymlinksInPath().path)
        )
        XCTAssertEqual(fixture.source.startCount, 1)
    }

    /// Every fixture Rust names renders on the detail screen with an enabled control for each
    /// available action and row action, and no enabled control for anything else
    ///
    /// An action is skipped only when every control Rust names for it is Android's, or when iOS
    /// draws it off the detail form (`isDrawnOffDetail`)
    func testEveryScreenFixtureDrawsItsAvailableActionsOnTheDetail() async throws {
        let fixtures = cloudBackupScreenFixtures()
        XCTAssertFalse(fixtures.isEmpty, "debug builds must ship the screen fixtures")
        try await waitForBootstrap()

        for fixture in fixtures {
            let recorder = try await renderDetail(fixture.screen)
            let actions = fixture.screen.actions

            for entry in actions.entries where fixture.screen.drawsOnIOS(entry.action) {
                let drawnEnabled = recorder.actions[entry.action]
                switch entry.availability {
                case .available:
                    guard !Self.isDrawnOffDetail(entry.action, on: fixture.screen) else { continue }
                    XCTAssertEqual(drawnEnabled, true, "\(fixture.name): \(entry.action) is available with no enabled control")
                case .unavailable:
                    XCTAssertNotEqual(drawnEnabled, true, "\(fixture.name): unavailable \(entry.action) is enabled")
                }
            }

            for (action, enabled) in recorder.actions where enabled {
                XCTAssertTrue(actions.isAvailable(action: action), "\(fixture.name): \(action) is enabled without Rust offering it")
            }

            for row in listedRows(fixture.screen) {
                for rowAction in row.actions {
                    let control = CloudBackupControlRecorder.RowControl(recordId: row.recordId, action: rowAction.action)
                    XCTAssertEqual(
                        recorder.rowActions[control],
                        rowAction.availability.isAvailable,
                        "\(fixture.name): row \(row.recordId) \(rowAction.action) doesn't match its availability"
                    )
                }
            }
        }
    }

    /// Every prompt fixture's alert draws a choice for each entry Rust sends for its actions, enabled
    /// only when available, and no choice Rust sent no entry for
    func testEveryPromptFixtureDrawsItsChoicesFromTheirEntries() async throws {
        let prompts = cloudBackupScreenFixtures().compactMap { fixture in
            promptChoices(fixture.screen.rootPrompt).map { (fixture, $0) }
        }
        XCTAssertGreaterThanOrEqual(prompts.count, 2, "the fixtures must include the Enable prompts")

        for (fixture, choices) in prompts {
            let recorder = try await render(VStack {
                CloudBackupPromptChoiceButtons(
                    choices: choices.offered(on: fixture.screen),
                    choose: { _ in },
                    cancel: {}
                )
            })

            for action in Set(choices.map(\.action)) {
                let availability = fixture.screen.actions.availability(action: action)
                XCTAssertEqual(
                    recorder.actions[action],
                    availability.map(\.isAvailable),
                    "\(fixture.name): the \(action) choice doesn't follow its entry"
                )
            }
        }
    }

    /// An alert has no room under its buttons, so a disabled choice's reason and a rejected
    /// choice's outcome are in its message
    func testPromptMessageCarriesTheReasonAndTheRejection() throws {
        let fixture = try XCTUnwrap(cloudBackupScreenFixtures().first { $0.name.hasSuffix(":enabling-existing-backup-prompt") })
        let running = CloudBackupRejection.operationRunning(operation: .restore)
        var screen = fixture.screen
        screen.actions = CloudBackupActions(entries: screen.actions.entries.map { entry in
            entry.action == .acceptEnablePromptCreateNew
                ? CloudBackupActionEntry(action: entry.action, availability: .unavailable(reason: running))
                : entry
        })
        screen.lastOutcome = CloudBackupLastOutcome(
            target: .action(.acceptEnablePromptUseExisting),
            outcome: .failed(message: "The passkey request failed")
        )

        let message = CloudBackupPromptChoice.existingBackup.offered(on: screen).message("Base", on: screen)

        XCTAssertEqual(message, "Base\n\n\(running.caption)\n\nThe passkey request failed")
    }

    /// An open delete confirmation follows the row's delete through each screen Rust sends
    func testOpenRowDeleteFollowsTheRowsDelete() throws {
        let target = CloudBackupRowTarget(recordId: "record-spending-old", control: .delete)
        let open = { (name: String) throws -> CloudBackupOpenRowDelete in
            let fixture = try XCTUnwrap(cloudBackupScreenFixtures().first { $0.name.hasSuffix(":\(name)") })
            return CloudBackupOpenRowDelete(target: target, delete: fixture.screen.rowAction(target: target))
        }

        let standard = try open("configured")
        let held = try open("configured-rows-held")
        let onlyCopy = try open("configured-extra-copy-only-copy")
        let gone = try open("configured-extra-copy-gone")

        XCTAssertEqual(CloudBackupOpenRowDelete.follow(from: standard, to: held), .keep)
        XCTAssertEqual(held.delete?.availability.isAvailable, false)
        XCTAssertEqual(CloudBackupOpenRowDelete.follow(from: standard, to: gone), .close)
        guard case let .delete(confirmation, consent) = onlyCopy.delete?.action else {
            return XCTFail("the only copy offers Delete")
        }
        XCTAssertEqual(
            CloudBackupOpenRowDelete.follow(from: standard, to: onlyCopy),
            .replace(recordId: "record-spending-old", consent: consent),
            "the replacing alert sends back the consent of the delete it shows"
        )
        XCTAssertEqual(
            confirmation,
            .mayBeOnlyCopy,
            "the replacing alert reads the stronger warning from the screen"
        )
    }

    /// The UI tests' identifier helper, which can't link Rust, names every element as Rust does
    func testUITestIdentifierHelperMatchesRust() {
        let elements = cloudBackupUiElements()
        XCTAssertFalse(elements.isEmpty)

        for element in elements {
            XCTAssertEqual(uiTestIdentifier(element), element.accessibilityIdentifier(), "\(element)")
        }

        // the UI tests name an action by its Swift case name
        for case let .action(action) in elements {
            XCTAssertEqual(String(describing: action), action.identifierName())
        }
    }

    private func promptChoices(_ prompt: CloudBackupRootPrompt) -> [CloudBackupPromptChoice]? {
        switch prompt {
        case .existingBackupFound: CloudBackupPromptChoice.existingBackup
        case let .passkeyChoice(intent): CloudBackupPasskeyChoicePresentation(intent: intent).choices
        case .none, .missingPasskeyReminder, .verification: nil
        }
    }

    // one exhaustive arm per element, so an element Rust adds fails to compile here; splitting it
    // to lower the count would need a default arm, which would hide that
    // swiftlint:disable:next cyclomatic_complexity
    private func uiTestIdentifier(_ element: CloudBackupUiElement) -> String {
        typealias Id = CloudBackupUIIdentifier

        switch element {
        case let .action(action): return Id.action(action.identifierName())
        case let .actionUnavailable(action): return Id.unavailable(action.identifierName())
        case let .actionConfirm(action): return Id.confirm(action.identifierName())
        case let .rowAction(target): return Id.row(target.recordId, target.control.identifierName())
        case let .rowConfirm(target): return Id.rowConfirm(target.recordId, target.control.identifierName())
        case let .walletRow(recordId): return Id.wallet(recordId)
        case .rowsUnavailable: return Id.rowsUnavailable
        case .lastOutcome: return Id.lastOutcome
        case .lastOutcomeDismiss: return Id.lastOutcomeDismiss
        case .healNotice: return Id.healNotice
        case .healNoticeTurnOff: return Id.healNoticeTurnOff
        case .turnOffPreparing: return Id.turnOffPreparing
        case .turnOffOffline: return Id.turnOffOffline
        case .turnOffOfflineTryAgain: return Id.turnOffOfflineTryAgain
        case .turnOffReview: return Id.turnOffReview
        case .turnOffContinue: return Id.turnOffContinue
        case .turnOffCheckAgain: return Id.turnOffCheckAgain
        case .turnOffFinal: return Id.turnOffFinal
        case .turnOffDraining: return Id.turnOffDraining
        case .finishingDisable: return Id.finishingDisable
        case .loadFailed: return Id.loadFailed
        case .corrupted: return Id.corrupted
        case .driveAccountSwitch: return Id.driveAccountSwitch
        case .inventoryChecking: return Id.inventoryChecking
        case .inventoryIncomplete: return Id.inventoryIncomplete
        case .recoverySupportCode: return Id.recoverySupportCode
        case .menu: return Id.menu
        case .promptExistingBackupFound: return Id.promptExistingBackupFound
        case .promptPasskeyChoice: return Id.promptPasskeyChoice
        case .promptPasskeyChoiceCreateNewPasskey: return Id.promptPasskeyChoiceCreateNewPasskey
        case .promptMissingPasskeyReminder: return Id.promptMissingPasskeyReminder
        case .promptVerification: return Id.promptVerification
        }
    }

    /// Actions whose control iOS draws somewhere other than the detail screen's form, and where
    private static let drawnOffDetail: [CloudBackupScreenAction: String] = [
        .enable: "the Settings row and the Enable sheet",
        .restoreFromCloudBackup: "onboarding",
        .cancelRestore: "the onboarding restore screen",
        .acceptEnablePromptUseExisting: "the root prompts",
        .acceptEnablePromptCreateNew: "the root prompts",
        .confirmPendingEnableCleanup: "the Enable sheet",
        .confirmDisable: "the Turn Off dialogs",
        .cancelDisable: "the Turn Off dialogs, and the detail only while Turn Off prepares or drains",
        .restoreFirst: "the Turn Off review dialog",
        .verifyFirst: "the Turn Off review dialog",
    ]

    /// Whether iOS draws an available action's control somewhere other than the detail form
    ///
    /// Discard is off the form only while Enable waits on the Existing Backup Found prompt or the
    /// passkey choice, where the root prompts' Cancel draws it. Saved Passkey Confirmation draws its
    /// Cancel on the form, so that one stays checked
    private static func isDrawnOffDetail(_ action: CloudBackupScreenAction, on screen: CloudBackupScreen) -> Bool {
        guard action == .discardPendingEnable else { return drawnOffDetail[action] != nil }
        guard case let .enabling(flow) = screen.body else { return false }

        switch flow {
        case .awaitingForceNewConfirmation, .awaitingPasskeyChoice: return true
        default: return false
        }
    }

    private func renderDetail(_ screen: CloudBackupScreen) async throws -> CloudBackupControlRecorder {
        try await render(Form {
            CloudBackupDetailBody(
                screen: screen,
                manager: .shared,
                presentationCoordinator: PresentationTransitionCoordinator<CloudBackupDetailPresentation>()
            )
        })
    }

    private func render(_ view: some View) async throws -> CloudBackupControlRecorder {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let recorder = CloudBackupControlRecorder()
        let window = UIWindow(windowScene: scene)

        // tall enough that the form builds every row, since it only builds rows near the screen
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 12000)
        window.rootViewController = UIHostingController(
            rootView: view.environment(\.cloudBackupControlRecorder, recorder)
        )
        window.isHidden = false
        defer { window.isHidden = true }

        window.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(300))
        return recorder
    }

    /// The shared manager spawns Rust actors, which need the app's bootstrap to have finished
    private func waitForBootstrap() async throws {
        let deadline = Date().addingTimeInterval(30)
        while bootstrapProgress() != .complete, Date() < deadline {
            try await Task.sleep(for: .milliseconds(100))
        }

        XCTAssertEqual(bootstrapProgress(), .complete, "the app never finished its bootstrap")
    }

    private func listedRows(_ screen: CloudBackupScreen) -> [CloudBackupListedRow] {
        guard case let .configured(configured) = screen.body else { return [] }

        return configured.sections.flatMap { section -> [CloudBackupListedRow] in
            switch section {
            case let .notOnDevice(notOnDevice): notOnDevice.rows
            case let .extraCopies(extraCopies): extraCopies.rows
            default: []
            }
        }
    }

    /// A review's Continue keeps its confirmation through a sync that runs while the review alert
    /// dismisses, and a sync for no dialog drops the queued confirmation too
    func testTurnOffSyncKeepsAQueuedConfirmationWhileTheReviewDismisses() throws {
        let coordinator = PresentationTransitionCoordinator<CloudBackupDetailPresentation>()
        coordinator.syncTurnOffDialog(.review(reviewId: 2))
        XCTAssertEqual(coordinator.currentPresentation?.item.turnOffDialog, .review(reviewId: 2))

        // Continue on the review
        coordinator.transition(to: .alert(.turnOff(.confirm(reviewId: 2))))
        let readiness = try XCTUnwrap(coordinator.readinessRequestID)

        // the scene comes back to active before the review alert has finished dismissing
        coordinator.syncTurnOffDialog(.review(reviewId: 2))
        coordinator.presenterDidBecomeReady(readiness)

        XCTAssertEqual(
            coordinator.currentPresentation?.item.turnOffDialog,
            .confirm(reviewId: 2),
            "the sync replaced the queued confirmation with the review"
        )

        // Rust leaves the review while the confirmation is queued behind another dismissal
        coordinator.transition(to: .alert(.turnOff(.confirm(reviewId: 2))))
        let next = try XCTUnwrap(coordinator.readinessRequestID)
        coordinator.syncTurnOffDialog(.none)
        coordinator.presenterDidBecomeReady(next)

        XCTAssertNil(coordinator.currentPresentation, "a dialog Rust no longer calls for came up")
        XCTAssertNil(coordinator.queuedPresentation)
    }

    private func writeEmptyFile(_ name: String, in directory: URL) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data().write(to: directory.appendingPathComponent(name))
    }
}
