import Foundation

// This file is compiled into both the scripted host app and the CoveCloudBackupUITests target,
// so the script a test writes and the state the host reports have one definition. It must only
// use Foundation, since the test target doesn't link CoveCore

/// File names and environment keys of the scripted host's control directory
///
/// A test names a fresh directory under `/tmp` in `COVE_CLOUD_BACKUP_UI_CONTROL_DIR`; the
/// simulator shares the host's `/tmp`, so both processes see the same files. The test writes
/// `script.json`, the host reads it on every provider call and writes `state.json` after every
/// call it records, and the test releases a hold or a gate by creating `release-<id>`
enum CloudBackupUITestControl {
    static let environmentKey = "COVE_CLOUD_BACKUP_UI_CONTROL_DIR"
    static let scriptFileName = "script.json"
    static let stateFileName = "state.json"
    static let keychainFileName = "keychain.json"
    /// Rust's `CSPP_PENDING_ENABLE_JOURNAL_KEY`, the keychain item that owns an unfinished enable
    static let pendingEnableJournalKey = "cspp::v1::pending_enable_journal"
    static let cloudDirectoryName = "cloud"
    /// The app's stdout and stderr, Rust logs included, appended across launches
    static let appLogFileName = "app.log"
    /// The cloud backup manager's state as the app holds it, a `CloudBackupUITestAppState`
    static let appStateFileName = "app-state.json"

    /// Gate that keeps fresh-enable uploads unreadable until the test creates its release file
    static let visibilityGate = "visibility"

    /// The credential id of the one passkey the controlled provider holds, which the fixture's
    /// master key is wrapped with
    static let passkeyCredentialId = "cove-cloud-backup-ui-test"

    static func releaseFileName(_ id: String) -> String {
        "release-\(id)"
    }

    /// Writes through a temporary file and a rename, since the other process polls these files
    /// and must never read a partial write
    static func writeAtomically(_ data: Data, to url: URL) throws {
        try data.write(to: url, options: .atomic)
    }
}

/// What the scripted host serves and where it holds, written by the test before each launch and
/// updated while the app runs
struct CloudBackupUITestScript: Codable, Equatable {
    enum Cloud: String, Codable {
        /// The generated fixture backup: its namespace, master key and wallets
        case fixture
        /// An empty cloud that accepts a new namespace and keeps uploads unreadable until the
        /// visibility gate is released
        case freshEnable
    }

    enum NamespaceResult: String, Codable {
        case empty
        case backupFound
        case offline
        case unavailable
    }

    enum WalletDownloadResult: String, Codable {
        case valid
        case corrupt
        case unavailable
    }

    enum ProviderError: String, Codable {
        case uploadFailed
        case offline
        case notAvailable
        /// The one upload error Cove doesn't retry on its own, so the wallet waits for Sync Now
        case quotaExceeded
    }

    enum PasskeyResult: String, Codable {
        case success
        case prePresentationFailure
        case postPresentationFailure
        case userCancelled
        case noCredential
    }

    enum HoldPoint: String, Codable {
        /// Before `listNamespaces` answers
        case namespaceList
        /// Before `listWalletFiles` answers
        case walletList
        /// Before `downloadWalletBackup` answers for `recordId`
        case walletDownload
        /// After `uploadMasterKeyBackup` stored the master key and before it returns
        case masterUploadReturn
        /// When `uploadWalletBackup` is called, before the cloud stores it; skipped calls count
        /// every wallet upload, whichever record it is for
        case walletUpload
        /// When `createPasskey` is called, before the passkey is registered
        case passkeyCreate
        /// When `authenticateWithPrf` is called, before the passkey answers, such as while a
        /// fresh enable confirms the passkey it just registered
        case passkeyAuthenticate
        /// Before the keychain deletes the pending enable journal, the last step of an enable's
        /// local promotion, after the new master key and passkey are committed
        case pendingEnableJournalDelete
    }

    /// A call that waits until the test creates `release-<id>`
    ///
    /// Each hold is claimed once across every launch that shares the control directory, so a
    /// relaunched app doesn't wait on a hold its previous process already entered
    struct Hold: Codable, Equatable {
        var id: String
        var point: HoldPoint
        var recordId: String?
        /// Matching calls that pass before the hold claims one
        var matchingCallsToSkip: Int = 0
    }

    enum UploadKind: String, Codable {
        case master
        case wallet
    }

    /// Fails one upload call, counted from one across every launch
    struct UploadFailure: Codable, Equatable {
        var kind: UploadKind
        var attempt: Int
        var error: ProviderError
    }

    var cloud: Cloud = .fixture

    /// Whether the scripted network is up
    ///
    /// While it is down every provider call fails as offline and the host reports the device as
    /// disconnected, so a test can drop the network without touching the simulator's, which the
    /// fake node needs
    var networkConnected = true

    /// Answers to `listNamespaces` in call order; the last one repeats
    var namespaceResults: [NamespaceResult] = [.backupFound]

    /// Fixture wallets the provider lists, or the three fixture wallets when nil
    ///
    /// The duplicate of Fixture Wallet One is listed only when named here
    var listedWalletRecordIds: [String]?

    /// Record id of the fixture's second backup of Fixture Wallet One under another wallet id
    static let duplicateWalletRecordId = "5be7eb6500dc9cd23536f3b86b3506cf5252fb0f23f1a5284eafc612764fb4d7"

    /// Answers to each fixture wallet's downloads in call order; the last one repeats
    var walletDownloadResults: [String: [WalletDownloadResult]] = [:]

    var holds: [Hold] = []
    var uploadFailures: [UploadFailure] = []

    /// Failures for the fixture cloud's uploads of wallets the fixture doesn't hold, such as one
    /// added on the device after a restore, in upload order across every launch; later uploads
    /// are accepted
    var newWalletUploadFailures: [ProviderError] = []

    /// Answers to each passkey call in call order; the last one repeats
    var createResults: [PasskeyResult] = [.success]
    var authenticateResults: [PasskeyResult] = [.success]
    var discoverResults: [PasskeyResult] = [.success]
}

/// Calls the host has seen and holds it has entered, across every launch in the control directory
struct CloudBackupUITestHostState: Codable, Equatable {
    struct Launch: Codable, Equatable {
        var processId: Int32
        var calls: [String: Int] = [:]
    }

    /// One upload the app asked the cloud to store, in call order
    struct Upload: Codable, Equatable {
        var kind: CloudBackupUITestScript.UploadKind
        var namespace: String
        /// Nil for the master key
        var recordId: String?
        /// False when the script failed it
        var accepted: Bool
        /// Whether it was a fixture-cloud upload of a wallet the fixture doesn't hold
        var isNewWallet = false
    }

    var launches: [Launch] = []
    var calls: [String: Int] = [:]
    var enteredHolds: [String] = []
    var uploads: [Upload] = []
    /// The connectivity the host last reported to Rust, nil before the app started reporting
    var reportedNetworkConnected: Bool?
    /// Hex credential ids the app asked the passkey provider to authenticate, in call order
    var authenticatedCredentialIds: [String] = []

    func count(_ call: CloudBackupUITestCall) -> Int {
        calls[call.key] ?? 0
    }

    /// Calls made by the latest launch only
    func countInLatestLaunch(_ call: CloudBackupUITestCall) -> Int {
        launches.last?.calls[call.key] ?? 0
    }

    /// Upload confirmation checks of the record checked most often, across every launch, so one
    /// verifier pass over several records never reads as polling twice
    var mostUploadStatusChecksOfOneRecord: Int {
        calls.filter { $0.key.hasPrefix("uploadStatus.") }.values.max() ?? 0
    }

    /// Downloads of every wallet record, across every launch
    var walletDownloadCount: Int {
        calls.filter { $0.key.hasPrefix("walletDownload.") }.values.reduce(0, +)
    }
}

/// Provider calls the host counts
enum CloudBackupUITestCall: Equatable {
    case namespaceList
    case walletList
    case walletListSnapshot
    case masterDownload
    case walletDownload(recordId: String)
    case masterUpload
    case walletUpload
    /// A fixture-cloud wallet upload of a wallet the fixture doesn't hold, also counted as `walletUpload`
    case newWalletUpload
    case uploadStatus(recordId: String)
    /// A namespace deletion, which disabling Cloud Backup makes once its writes have drained
    case namespaceDelete
    case passkeyCreate
    case passkeyAuthenticate
    case passkeyDiscover
    /// A provider call refused because the scripted network is down; the call itself is not counted
    case offlineRejection

    var key: String {
        switch self {
        case .namespaceList: "namespaceList"
        case .walletList: "walletList"
        case .walletListSnapshot: "walletListSnapshot"
        case .masterDownload: "masterDownload"
        case let .walletDownload(recordId): "walletDownload.\(recordId)"
        case .masterUpload: "masterUpload"
        case .walletUpload: "walletUpload"
        case .newWalletUpload: "newWalletUpload"
        case let .uploadStatus(recordId): "uploadStatus.\(recordId)"
        case .namespaceDelete: "namespaceDelete"
        case .passkeyCreate: "passkeyCreate"
        case .passkeyAuthenticate: "passkeyAuthenticate"
        case .passkeyDiscover: "passkeyDiscover"
        case .offlineRejection: "offlineRejection"
        }
    }
}

/// The cloud backup manager's state as the app holds it, which the host writes to
/// `app-state.json` whenever it changes
///
/// The Cloud Backup form builds only the rows near the screen, so a test checks what the app holds
/// here and what a person sees on screen
struct CloudBackupUITestAppState: Codable, Equatable {
    /// `none`, `notLoaded`, `checking`, `complete` (provider confirmed), `completeUnconfirmed` or `failed`
    var detail: String
    /// `idle`, `operating`, `warning` or `failed`
    var operation: String
    /// Cloud-only wallet record ids, empty unless the cloud-only list is loaded
    var cloudOnly: [String]
    var cloudOnlyIncomplete: Bool
    /// Whether a cloud-only row may restore or delete its wallet
    var inventoryReady: Bool
    /// Whether the provider confirmed the whole inventory, which Restore All needs
    var inventoryComplete: Bool
    var inventoryError: String?
    /// The Restore All state, such as `startAvailable(2)` or `running(1/2)`
    var restoreAll: String
    /// The verification case, such as `verified` or `awaitingUploadConfirmation`, or `none`
    var verification: String
    /// Whether verification waits for the provider to confirm uploads
    var pendingUploadVerification: Bool
    /// `idle`, `syncing`, `blocked`, `failed` or `none`
    var sync: String
    var syncError: String?
    /// The on-device wallets the detail lists, up to date first, then those that need a sync
    var wallets: [Wallet]
    /// The lifecycle case, such as `configured`, `enabling` or `pendingEnableRecovery`
    var lifecycle: String
    /// The Settings row status case without its message, such as `confirming`, `active` or
    /// `settingUp`
    var settingsRow: String

    struct Wallet: Codable, Equatable {
        var recordId: String
        var name: String
        /// The sync status, such as `confirmed`, `failed` or `dirty`
        var status: String
        var needsSync: Bool
    }
}
