import CoveCore
import Foundation

/// Cloud storage driven by the UI test's control directory instead of a fixed scenario
///
/// The test's script picks the fixture backup or an empty cloud for a fresh enable, and adds
/// holds, failures, delayed visibility and a dropped network. Uploads are kept as files in the
/// control directory, so a relaunched app finds what its killed process uploaded
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
        let failure = script.uploadFailures.first { $0.kind == .master && $0.attempt == attempt }?.error
        try recordUpload(script, kind: .master, failure: failure, namespace: namespace, recordId: nil)
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
        let isNewWallet = script.cloud == .fixture && !isFixtureWallet(recordId)
        let failure: CloudBackupUITestScript.ProviderError? = if isNewWallet {
            Self.failure(at: controller.record(.newWalletUpload), in: script.newWalletUploadFailures)
        } else {
            script.uploadFailures.first { $0.kind == .wallet && $0.attempt == attempt }?.error
        }

        try recordUpload(
            script,
            kind: .wallet,
            failure: failure,
            namespace: namespace,
            recordId: recordId,
            isNewWallet: isNewWallet
        )

        // a hold here proves which action made the upload, since it can't return until released
        await controller.holdIfScripted(at: .walletUpload, recordId: recordId, matchingCallIndex: attempt - 1)
        try validateWritable(namespace: namespace, script: script)
        try store.storeWallet(namespace: namespace, recordId: recordId, data: data)
    }

    func downloadMasterKeyBackup(
        namespace: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> Data {
        try throwIfOffline()
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

            guard !store.isNamespaceDeleted(namespace) else {
                throw CloudStorageError.NotFound("cloud backup was deleted")
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
        try throwIfOffline()
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
                  !store.isWalletDeleted(namespace: namespace, recordId: recordId)
            else { throw CloudStorageError.NotFound("cloud backup is not available") }

            guard let wallet = fixtureWallet(recordId) else {
                // a wallet added on the device is served once the cloud accepted its upload
                guard let uploaded = store.wallet(namespace: namespace, recordId: recordId) else {
                    throw CloudStorageError.NotFound("cloud backup is not available")
                }

                return uploaded
            }

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
        try throwIfOffline()

        // the fixture cloud falls back to built-in records, so a deletion leaves a marker
        store.deleteWallet(namespace: namespace, recordId: recordId)
    }

    func deleteNamespace(namespace: String, policy _: CloudAccessPolicy) async throws {
        try throwIfOffline()
        controller.record(.namespaceDelete)
        store.deleteNamespace(namespace)
    }

    func listNamespaces(policy _: CloudAccessPolicy) async throws -> [String] {
        try throwIfOffline()
        let index = controller.record(.namespaceList)
        await controller.holdIfScripted(at: .namespaceList, matchingCallIndex: index)
        let script = controller.script()

        let results = script.namespaceResults.isEmpty ? [.backupFound] : script.namespaceResults
        switch results[min(index, results.count - 1)] {
        case .empty:
            // a fresh-enable cloud lists what the app uploaded once it is visible
            guard script.cloud == .freshEnable, isVisible else { return [] }

            return store.namespaces()
        case .backupFound:
            guard script.cloud == .fixture else {
                fatalError("a fresh-enable cloud has no fixture backup to find; script .empty instead")
            }

            return store.isNamespaceDeleted(fixture.namespace) ? [] : [fixture.namespace]
        case .offline:
            throw CloudStorageError.Offline("scripted provider is offline")
        case .unavailable:
            throw CloudStorageError.NotAvailable("scripted provider is unavailable")
        }
    }

    func listWalletFiles(namespace: String, policy _: CloudAccessPolicy) async throws -> [String] {
        try throwIfOffline()
        let index = controller.record(.walletList)
        await controller.holdIfScripted(at: .walletList, matchingCallIndex: index)

        return try walletFileNames(namespace: namespace)
    }

    func listWalletFilesSnapshot(
        namespace: String,
        policy _: CloudAccessPolicy
    ) async throws -> CloudStorageInventorySnapshot {
        try throwIfOffline()
        controller.record(.walletListSnapshot)

        return try CloudStorageInventorySnapshot(names: walletFileNames(namespace: namespace), isComplete: true)
    }

    func isBackupUploaded(
        namespace: String,
        recordId: String,
        locations _: [RemoteBackupLocation],
        policy _: CloudAccessPolicy
    ) async throws -> CloudBackupUploadStatus {
        try throwIfOffline()
        controller.record(.uploadStatus(recordId: recordId))

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
            // a backup the fixture doesn't hold exists only once the cloud accepted its upload,
            // so a rejected one is reported missing, as Google Drive does on Android
            let exists =
                if recordId == Self.masterKeyRecordId {
                    !store.isNamespaceDeleted(namespace)
                } else {
                    !store.isWalletDeleted(namespace: namespace, recordId: recordId)
                        && (isFixtureWallet(recordId) || store.wallet(namespace: namespace, recordId: recordId) != nil)
                }

            return namespace == fixture.namespace && exists ? .uploaded : .notFound
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
            guard namespace == fixture.namespace, !store.isNamespaceDeleted(namespace) else {
                throw CloudStorageError.NotFound("cloud backup is not available")
            }

            // wallets added on the device are listed once the cloud accepted their uploads
            let listed = script.listedWalletRecordIds ?? fixture.wallets.map(\.recordId)
            let uploaded = store.walletRecordIds(namespace: namespace).filter { !isFixtureWallet($0) }
            return (listed + uploaded.sorted())
                .filter { !store.isWalletDeleted(namespace: namespace, recordId: $0) }
                .map(Self.walletFileName)
        }
    }

    private func fixtureWallet(_ recordId: String) -> CloudBackupFixture.Wallet? {
        fixture.walletsByRecordId[recordId] ?? [fixture.duplicateWallet].first { $0.recordId == recordId }
    }

    private func isFixtureWallet(_ recordId: String) -> Bool {
        fixtureWallet(recordId) != nil
    }

    /// Fails a call while the scripted network is down, as a real provider does, counting the
    /// refusal instead of the call, so call counts show only calls the provider served
    ///
    /// `overallSyncHealth` is left alone, since iCloud answers it from the files on the device
    private func throwIfOffline() throws {
        guard !controller.script().networkConnected else { return }

        controller.record(.offlineRejection)
        throw CloudStorageError.Offline("scripted network is offline")
    }

    /// Records the upload, and throws the script's failure or the dropped network when either
    /// fails it
    private func recordUpload(
        _ script: CloudBackupUITestScript,
        kind: CloudBackupUITestScript.UploadKind,
        failure: CloudBackupUITestScript.ProviderError?,
        namespace: String,
        recordId: String?,
        isNewWallet: Bool = false
    ) throws {
        let accepted = failure == nil && script.networkConnected
        controller.recordUpload(.init(
            kind: kind,
            namespace: namespace,
            recordId: recordId,
            accepted: accepted,
            isNewWallet: isNewWallet
        ))

        if !script.networkConnected {
            controller.record(.offlineRejection)
            throw CloudStorageError.Offline("scripted network is offline")
        }
        guard let failure else { return }

        switch failure {
        case .uploadFailed:
            throw CloudStorageError.UploadFailed("scripted \(kind.rawValue) upload failure")
        case .offline:
            throw CloudStorageError.Offline("scripted provider is offline")
        case .notAvailable:
            throw CloudStorageError.NotAvailable("scripted provider is unavailable")
        case .quotaExceeded:
            throw CloudStorageError.QuotaExceeded
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

    private static func failure(
        at index: Int,
        in failures: [CloudBackupUITestScript.ProviderError]
    ) -> CloudBackupUITestScript.ProviderError? {
        index < failures.count ? failures[index] : nil
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
