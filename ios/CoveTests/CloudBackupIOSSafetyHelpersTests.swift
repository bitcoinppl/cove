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

    func testCatastrophicProbeMappingDistinguishesInconclusiveStates() {
        XCTAssertEqual(CatastrophicErrorView.cloudProbeState(hasBackup: true), .available)
        XCTAssertEqual(CatastrophicErrorView.cloudProbeState(hasBackup: false), .noBackup)
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(error: .Offline("offline")),
            .offline("offline")
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(error: .NotAvailable("icloud unavailable")),
            .inconclusive("icloud unavailable")
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(error: .AuthorizationRequired("auth required")),
            .inconclusive("auth required")
        )
        XCTAssertEqual(
            CatastrophicErrorView.cloudProbeState(error: .DownloadFailed("bad data")),
            .unreadable("bad data")
        )

        XCTAssertTrue(CatastrophicErrorView.CloudProbeState.inconclusive("cold metadata").allowsRestoreAttempt)
        XCTAssertTrue(CatastrophicErrorView.CloudProbeState.unreadable("bad data").allowsRestoreAttempt)
        XCTAssertTrue(CatastrophicErrorView.CloudProbeState.offline("offline").allowsRetry)
        XCTAssertFalse(CatastrophicErrorView.CloudProbeState.offline("offline").allowsRestoreAttempt)
        XCTAssertFalse(CatastrophicErrorView.CloudProbeState.noBackup.allowsRestoreAttempt)
    }

    func testSettingsRowDoesNotShowActiveForUnhealthySyncHealth() {
        XCTAssertEqual(
            cloudBackupSettingsRowStatus(
                isUnverified: false,
                hasPendingUploadVerification: false,
                isVerificationStale: false,
                syncHealth: .allUploaded
            ),
            .active
        )
        XCTAssertEqual(
            cloudBackupSettingsRowStatus(
                isUnverified: false,
                hasPendingUploadVerification: false,
                isVerificationStale: false,
                syncHealth: .unavailable
            ),
            .unavailable
        )
        XCTAssertEqual(
            cloudBackupSettingsRowStatus(
                isUnverified: false,
                hasPendingUploadVerification: false,
                isVerificationStale: false,
                syncHealth: .authorizationRequired("auth required")
            ),
            .authorizationRequired("auth required")
        )
        XCTAssertEqual(
            cloudBackupSettingsRowStatus(
                isUnverified: false,
                hasPendingUploadVerification: false,
                isVerificationStale: false,
                syncHealth: .failed("sync failed")
            ),
            .failed("sync failed")
        )
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

    private func assertInvalidNamespace(_ namespace: String) {
        XCTAssertThrowsError(try ICloudDriveHelper.shared.validateNamespace(namespace)) { error in
            guard case CloudStorageError.InvalidNamespace = error else {
                XCTFail("expected InvalidNamespace, got \(error)")
                return
            }
        }
    }
}
