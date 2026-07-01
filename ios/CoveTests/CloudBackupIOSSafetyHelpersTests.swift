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
            CatastrophicErrorView.cloudProbeState(result: .noBackupFound(provider: .iCloudDrive)),
            .noBackup
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(result: .offline(provider: .iCloudDrive)),
            .offline
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(
                result: .inconclusive(provider: .iCloudDrive, reason: .providerUnavailable)
            ),
            .inconclusive(provider: .iCloudDrive, reason: .providerUnavailable)
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(
                result: .inconclusive(provider: .iCloudDrive, reason: .authorizationRequired)
            ),
            .inconclusive(provider: .iCloudDrive, reason: .authorizationRequired)
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(result: .unreadable),
            .unreadable
        )

        XCTAssertFalse(
            CatastrophicErrorView.CloudProbeState
                .inconclusive(provider: .iCloudDrive, reason: .unknown)
                .allowsRestoreAttempt
        )
        XCTAssertFalse(CatastrophicErrorView.CloudProbeState.unreadable.allowsRestoreAttempt)
        XCTAssertTrue(CatastrophicErrorView.CloudProbeState.available.allowsRestoreAttempt)
        XCTAssertTrue(CatastrophicErrorView.CloudProbeState.offline.allowsRetry)
        XCTAssertFalse(CatastrophicErrorView.CloudProbeState.offline.allowsRestoreAttempt)
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
            .authorizationRequired,
            .unavailable,
            .failed,
        ]

        for state in unhealthyStates {
            XCTAssertNotEqual(cloudBackupDetailHeaderTitle(syncHealth: state), "Cloud Backup Active")
            XCTAssertNotEqual(cloudBackupDetailHeaderIconName(syncHealth: state), "checkmark.icloud.fill")
        }
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
