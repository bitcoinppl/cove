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
    static let cloudDirectoryName = "cloud"
    /// The app's stdout and stderr, Rust logs included, appended across launches
    static let appLogFileName = "app.log"

    /// Gate that keeps fresh-enable uploads unreadable until the test creates its release file
    static let visibilityGate = "visibility"

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

    var launches: [Launch] = []
    var calls: [String: Int] = [:]
    var enteredHolds: [String] = []

    func count(_ call: CloudBackupUITestCall) -> Int {
        calls[call.key] ?? 0
    }

    /// Calls made by the latest launch only
    func countInLatestLaunch(_ call: CloudBackupUITestCall) -> Int {
        launches.last?.calls[call.key] ?? 0
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
    case uploadStatus
    case passkeyCreate
    case passkeyAuthenticate
    case passkeyDiscover

    var key: String {
        switch self {
        case .namespaceList: "namespaceList"
        case .walletList: "walletList"
        case .walletListSnapshot: "walletListSnapshot"
        case .masterDownload: "masterDownload"
        case let .walletDownload(recordId): "walletDownload.\(recordId)"
        case .masterUpload: "masterUpload"
        case .walletUpload: "walletUpload"
        case .uploadStatus: "uploadStatus"
        case .passkeyCreate: "passkeyCreate"
        case .passkeyAuthenticate: "passkeyAuthenticate"
        case .passkeyDiscover: "passkeyDiscover"
        }
    }
}
