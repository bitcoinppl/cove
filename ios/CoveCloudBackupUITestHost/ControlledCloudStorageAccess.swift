import CoveCore
import Foundation

/// Cloud storage driven by the UI test's control directory instead of a fixed scenario
///
/// The test's script picks the fixture backup or an empty cloud for a fresh enable, and adds
/// holds, failures and delayed visibility. Uploads are kept as files in the control directory, so
/// a relaunched app finds what its killed process uploaded
final class ControlledCloudStorageAccess: CloudStorageAccess, @unchecked Sendable {
    private static let masterKeyRecordId = "cspp-master-key-v1"

    private let controller: CloudBackupUITestController
    private let store: CloudBackupUITestCloudStore
    private let fixture = CloudBackupFixture.load()

    init(controller: CloudBackupUITestController) {
        self.controller = controller
        store = CloudBackupUITestCloudStore(root: controller.cloudDirectory)
    }

    private var isVisible: Bool {
        controller.isReleased(CloudBackupUITestControl.visibilityGate)
    }

    func uploadMasterKeyBackup(
        namespace: String,
        location _: RemoteBackupLocation,
        data: Data,
        policy _: CloudAccessPolicy
    ) async throws {
        let attempt = controller.record(.masterUpload) + 1
        let script = controller.script()
        try failIfScripted(script, kind: .master, attempt: attempt)
        try validateWritable(namespace: namespace, script: script)
        try store.storeMaster(namespace: namespace, data: data)

        // the provider has accepted the write by now, so a kill here leaves the master in the cloud
        await controller.holdIfScripted(at: .masterUploadReturn, matchingCallIndex: attempt - 1)
    }

    func uploadWalletBackup(
        namespace: String,
        recordId: String,
        location _: RemoteBackupLocation,
        data: Data,
        policy _: CloudAccessPolicy
    ) async throws {
        let attempt = controller.record(.walletUpload) + 1
        let script = controller.script()
        try failIfScripted(script, kind: .wallet, attempt: attempt)
        try validateWritable(namespace: namespace, script: script)
        try store.storeWallet(namespace: namespace, recordId: recordId, data: data)
    }

    func downloadMasterKeyBackup(
        namespace: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        controller.record(.masterDownload)

        switch controller.script().cloud {
        case .freshEnable:
            guard isVisible, let master = store.master(namespace: namespace) else {
                throw CloudStorageError.NotFound("master key backup is not visible yet")
            }

            return master

        case .fixture:
            guard namespace == fixture.namespace else {
                throw CloudStorageError.NotFound("cloud backup is not available")
            }

            return store.master(namespace: namespace) ?? fixture.masterData
        }
    }

    func downloadWalletBackup(
        namespace: String,
        recordId: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        let index = controller.record(.walletDownload(recordId: recordId))
        await controller.holdIfScripted(at: .walletDownload, recordId: recordId, matchingCallIndex: index)
        let script = controller.script()

        switch script.cloud {
        case .freshEnable:
            guard isVisible, let wallet = store.wallet(namespace: namespace, recordId: recordId) else {
                throw CloudStorageError.NotFound("wallet backup is not visible yet")
            }

            return wallet

        case .fixture:
            guard namespace == fixture.namespace,
                  let wallet = fixture.walletsByRecordId[recordId]
                  ?? [fixture.duplicateWallet].first(where: { $0.recordId == recordId })
            else { throw CloudStorageError.NotFound("cloud backup is not available") }

            let results = script.walletDownloadResults[recordId] ?? []
            switch results.isEmpty ? .valid : results[min(index, results.count - 1)] {
            case .valid:
                return store.wallet(namespace: namespace, recordId: recordId) ?? wallet.data
            case .corrupt:
                return Self.corruptingCiphertext(of: wallet.data)
            case .unavailable:
                throw CloudStorageError.NotAvailable("scripted fixture wallet is unavailable")
            }
        }
    }

    func deleteWalletBackup(
        namespace: String,
        recordId: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws {
        store.deleteWallet(namespace: namespace, recordId: recordId)
    }

    func deleteNamespace(namespace: String, policy _: CloudAccessPolicy) async throws {
        store.deleteNamespace(namespace)
    }

    func listNamespaces(policy _: CloudAccessPolicy) async throws -> [String] {
        let index = controller.record(.namespaceList)
        await controller.holdIfScripted(at: .namespaceList, matchingCallIndex: index)
        let script = controller.script()

        if script.cloud == .freshEnable {
            return isVisible ? store.namespaces() : []
        }

        let results = script.namespaceResults.isEmpty ? [.backupFound] : script.namespaceResults
        switch results[min(index, results.count - 1)] {
        case .empty:
            return []
        case .backupFound:
            return [fixture.namespace]
        case .offline:
            throw CloudStorageError.Offline("scripted provider is offline")
        case .unavailable:
            throw CloudStorageError.NotAvailable("scripted provider is unavailable")
        }
    }

    func listWalletFiles(namespace: String, policy _: CloudAccessPolicy) async throws -> [String] {
        let index = controller.record(.walletList)
        await controller.holdIfScripted(at: .walletList, matchingCallIndex: index)

        return try walletFileNames(namespace: namespace)
    }

    func listWalletFilesSnapshot(
        namespace: String,
        policy _: CloudAccessPolicy
    ) async throws -> CloudStorageInventorySnapshot {
        controller.record(.walletListSnapshot)

        return try CloudStorageInventorySnapshot(names: walletFileNames(namespace: namespace), isComplete: true)
    }

    func isBackupUploaded(
        namespace: String,
        recordId: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> CloudBackupUploadStatus {
        controller.record(.uploadStatus)

        switch controller.script().cloud {
        case .freshEnable:
            guard isVisible else { return .pending }

            let exists =
                if recordId == Self.masterKeyRecordId {
                    store.master(namespace: namespace) != nil
                } else {
                    store.wallet(namespace: namespace, recordId: recordId) != nil
                }

            return exists ? .uploaded : .notFound

        case .fixture:
            return namespace == fixture.namespace ? .uploaded : .notFound
        }
    }

    func overallSyncHealth(policy _: CloudAccessPolicy) async -> CloudSyncHealth {
        switch controller.script().cloud {
        case .freshEnable:
            if store.namespaces().isEmpty { return .noFiles }

            return isVisible ? .allUploaded : .uploading

        case .fixture:
            return .allUploaded
        }
    }

    private func walletFileNames(namespace: String) throws -> [String] {
        let script = controller.script()

        switch script.cloud {
        case .freshEnable:
            guard isVisible else { return [] }

            return store.walletRecordIds(namespace: namespace).map(Self.walletFileName)

        case .fixture:
            guard namespace == fixture.namespace else {
                throw CloudStorageError.NotFound("cloud backup is not available")
            }

            let listed = script.listedWalletRecordIds ?? fixture.wallets.map(\.recordId)
            return listed.map(Self.walletFileName)
        }
    }

    private func failIfScripted(
        _ script: CloudBackupUITestScript,
        kind: CloudBackupUITestScript.UploadKind,
        attempt: Int
    ) throws {
        guard let failure = script.uploadFailures.first(where: { $0.kind == kind && $0.attempt == attempt })
        else { return }

        switch failure.error {
        case .uploadFailed:
            throw CloudStorageError.UploadFailed("scripted \(kind.rawValue) upload failure")
        case .offline:
            throw CloudStorageError.Offline("scripted provider is offline")
        case .notAvailable:
            throw CloudStorageError.NotAvailable("scripted provider is unavailable")
        }
    }

    private func validateWritable(namespace: String, script: CloudBackupUITestScript) throws {
        switch script.cloud {
        case .freshEnable:
            // a fresh enable picks a random 32-character lowercase hex namespace
            guard namespace.count == 32, namespace.allSatisfy({ $0.isHexDigit && !$0.isUppercase }) else {
                throw CloudStorageError.InvalidNamespace("unexpected fresh namespace")
            }

        case .fixture:
            guard namespace == fixture.namespace else {
                throw CloudStorageError.NotFound("cloud backup is not available")
            }
        }
    }

    private static func walletFileName(_ recordId: String) -> String {
        "wallet-\(recordId).json"
    }

    /// Flips one ciphertext character, so the blob parses but fails to decrypt
    private static func corruptingCiphertext(of data: Data) -> Data {
        guard var json = String(data: data, encoding: .utf8),
              let marker = json.range(of: "\"ciphertext\":\"")
        else { fatalError("fixture wallet ciphertext is missing") }

        let index = marker.upperBound
        let replacement: Character = json[index] == "A" ? "B" : "A"
        json.replaceSubrange(index ... index, with: String(replacement))

        return Data(json.utf8)
    }
}
