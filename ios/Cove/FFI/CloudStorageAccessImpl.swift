import CloudKit

@_exported import CoveCore
import Foundation

final class CloudStorageAccessImpl: CloudStorageAccess, @unchecked Sendable {
    private let container = CKContainer(identifier: "iCloud.com.covebitcoinwallet")
    private var db: CKDatabase {
        container.privateCloudDatabase
    }

    private static let recordType = "CSPPBackup"
    private static let dataField = "data"

    // MARK: - Upload

    func uploadMasterKeyBackup(data: [UInt8]) throws {
        try uploadRecord(recordId: "cspp-master-key-v1", data: Data(data))
    }

    func uploadWalletBackup(recordId: String, data: [UInt8]) throws {
        try uploadRecord(recordId: recordId, data: Data(data))
    }

    func uploadManifest(data: [UInt8]) throws {
        try uploadRecord(recordId: "cspp-manifest-v1", data: Data(data))
    }

    // MARK: - Download

    func downloadMasterKeyBackup() throws -> [UInt8] {
        try downloadRecord(recordId: "cspp-master-key-v1")
    }

    func downloadWalletBackup(recordId: String) throws -> [UInt8] {
        try downloadRecord(recordId: recordId)
    }

    func downloadManifest() throws -> [UInt8] {
        try downloadRecord(recordId: "cspp-manifest-v1")
    }

    // MARK: - Presence check

    func hasCloudBackup() throws -> Bool {
        let recordID = CKRecord.ID(recordName: "cspp-manifest-v1")
        let semaphore = DispatchSemaphore(value: 0)
        var fetchResult: Result<Bool, CloudStorageError>!

        db.fetch(withRecordID: recordID) { record, error in
            if let record {
                _ = record // record exists
                fetchResult = .success(true)
            } else if let ckError = error as? CKError {
                switch ckError.code {
                case .unknownItem:
                    fetchResult = .success(false)
                case .networkUnavailable, .networkFailure, .serviceUnavailable:
                    fetchResult = .failure(
                        .notAvailable(ckError.localizedDescription)
                    )
                default:
                    fetchResult = .failure(
                        .downloadFailed(ckError.localizedDescription)
                    )
                }
            } else if let error {
                fetchResult = .failure(.notAvailable(error.localizedDescription))
            } else {
                fetchResult = .success(false)
            }
            semaphore.signal()
        }

        semaphore.wait()
        return try fetchResult.get()
    }

    // MARK: - Private helpers

    private func uploadRecord(recordId: String, data: Data) throws {
        let record = CKRecord(
            recordType: Self.recordType,
            recordID: CKRecord.ID(recordName: recordId)
        )
        record[Self.dataField] = data as CKRecordValue

        let semaphore = DispatchSemaphore(value: 0)
        var uploadError: CloudStorageError?

        let operation = CKModifyRecordsOperation(
            recordsToSave: [record],
            recordIDsToDelete: nil
        )
        operation.savePolicy = .changedKeys
        operation.modifyRecordsResultBlock = { result in
            if case let .failure(error) = result {
                if let ckError = error as? CKError, ckError.code == .quotaExceeded {
                    uploadError = .quotaExceeded
                } else {
                    uploadError = .uploadFailed(error.localizedDescription)
                }
            }
            semaphore.signal()
        }

        db.add(operation)
        semaphore.wait()

        if let error = uploadError {
            throw error
        }
    }

    private func downloadRecord(recordId: String) throws -> [UInt8] {
        let recordID = CKRecord.ID(recordName: recordId)
        let semaphore = DispatchSemaphore(value: 0)
        var fetchResult: Result<[UInt8], CloudStorageError>!

        db.fetch(withRecordID: recordID) { record, error in
            if let record, let data = record[Self.dataField] as? Data {
                fetchResult = .success(Array(data))
            } else if let ckError = error as? CKError {
                switch ckError.code {
                case .unknownItem:
                    fetchResult = .failure(.notFound(recordId))
                case .networkUnavailable, .networkFailure, .serviceUnavailable:
                    fetchResult = .failure(
                        .notAvailable(ckError.localizedDescription)
                    )
                default:
                    fetchResult = .failure(
                        .downloadFailed(ckError.localizedDescription)
                    )
                }
            } else if let error {
                fetchResult = .failure(.downloadFailed(error.localizedDescription))
            } else {
                fetchResult = .failure(.notFound(recordId))
            }
            semaphore.signal()
        }

        semaphore.wait()
        return try fetchResult.get()
    }
}
