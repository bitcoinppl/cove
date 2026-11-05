import Foundation

/// The host's side of the control directory a UI test names in `COVE_CLOUD_BACKUP_UI_CONTROL_DIR`
///
/// It reads the test's script on every call, so a test can change what the provider serves while
/// the app runs, counts calls in `state.json`, and parks a call on a hold until the test releases
/// it. State loaded from an earlier launch is kept, so a relaunched app keeps counting and never
/// re-enters a hold its previous process claimed
final class CloudBackupUITestController: @unchecked Sendable {
    let directory: URL
    private let lock = NSLock()
    private var state: CloudBackupUITestHostState
    private var claimedHolds: Set<String>

    static func fromEnvironment(
        _ environment: [String: String] = ProcessInfo.processInfo.environment
    ) -> CloudBackupUITestController? {
        guard let path = environment[CloudBackupUITestControl.environmentKey], !path.isEmpty else {
            return nil
        }

        return CloudBackupUITestController(directory: URL(fileURLWithPath: path, isDirectory: true))
    }

    init(directory: URL) {
        self.directory = directory
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        let stateURL = directory.appendingPathComponent(CloudBackupUITestControl.stateFileName)
        var state = (try? Data(contentsOf: stateURL))
            .flatMap { try? JSONDecoder().decode(CloudBackupUITestHostState.self, from: $0) }
            ?? CloudBackupUITestHostState()
        state.launches.append(.init(processId: ProcessInfo.processInfo.processIdentifier))

        self.state = state
        claimedHolds = Set(state.enteredHolds)
        persistState()
        Self.redirectOutput(to: directory.appendingPathComponent(CloudBackupUITestControl.appLogFileName))
    }

    /// Sends stdout and stderr to the control directory, so a failed test can attach what the
    /// app logged, since the simulator clone and its logs are gone once the test ends
    private static func redirectOutput(to url: URL) {
        let descriptor = open(url.path, O_WRONLY | O_CREAT | O_APPEND, 0o644)
        guard descriptor >= 0 else { return }

        dup2(descriptor, STDOUT_FILENO)
        dup2(descriptor, STDERR_FILENO)
        close(descriptor)
    }

    var keychainURL: URL {
        directory.appendingPathComponent(CloudBackupUITestControl.keychainFileName)
    }

    var cloudDirectory: URL {
        directory.appendingPathComponent(CloudBackupUITestControl.cloudDirectoryName, isDirectory: true)
    }

    /// The test's current script, read fresh so updates while the app runs take effect
    func script() -> CloudBackupUITestScript {
        let url = directory.appendingPathComponent(CloudBackupUITestControl.scriptFileName)
        guard let data = try? Data(contentsOf: url) else { return CloudBackupUITestScript() }

        do {
            return try JSONDecoder().decode(CloudBackupUITestScript.self, from: data)
        } catch {
            fatalError("invalid cloud backup UI test script: \(error)")
        }
    }

    func isReleased(_ id: String) -> Bool {
        FileManager.default.fileExists(
            atPath: directory.appendingPathComponent(CloudBackupUITestControl.releaseFileName(id)).path
        )
    }

    /// Counts a call and returns how many of its kind came before it, across every launch
    @discardableResult
    func record(_ call: CloudBackupUITestCall) -> Int {
        lock.withLock {
            let previous = state.calls[call.key] ?? 0
            state.calls[call.key] = previous + 1
            state.launches[state.launches.count - 1].calls[call.key, default: 0] += 1
            persistStateLocked()

            return previous
        }
    }

    /// Waits on the first unclaimed hold at `point` once its skipped calls have passed
    ///
    /// `matchingCallIndex` is the zero-based index of this call among the calls the hold matches
    func holdIfScripted(
        at point: CloudBackupUITestScript.HoldPoint,
        recordId: String? = nil,
        matchingCallIndex: Int
    ) async {
        let hold = lock.withLock { () -> CloudBackupUITestScript.Hold? in
            let candidate = script().holds.first { hold in
                hold.point == point
                    && (hold.recordId == nil || hold.recordId == recordId)
                    && !claimedHolds.contains(hold.id)
                    && matchingCallIndex >= hold.matchingCallsToSkip
            }
            guard let candidate else { return nil }

            claimedHolds.insert(candidate.id)
            state.enteredHolds.append(candidate.id)
            persistStateLocked()

            return candidate
        }
        guard let hold else { return }

        while !isReleased(hold.id) {
            try? await Task.sleep(for: .milliseconds(50))
        }
    }

    private func persistState() {
        lock.withLock { persistStateLocked() }
    }

    private func persistStateLocked() {
        let url = directory.appendingPathComponent(CloudBackupUITestControl.stateFileName)
        do {
            let data = try JSONEncoder().encode(state)
            try CloudBackupUITestControl.writeAtomically(data, to: url)
        } catch {
            fatalError("failed to write cloud backup UI test state: \(error)")
        }
    }
}

/// Uploaded backups kept as files in the control directory, so they survive a relaunch the way a
/// cloud provider's copy survives the app being killed
final class CloudBackupUITestCloudStore: @unchecked Sendable {
    private let root: URL
    private let lock = NSLock()

    init(root: URL) {
        self.root = root
    }

    func storeMaster(namespace: String, data: Data) throws {
        try lock.withLock { try write(data, to: masterURL(namespace)) }
    }

    func storeWallet(namespace: String, recordId: String, data: Data) throws {
        try lock.withLock { try write(data, to: walletURL(namespace, recordId)) }
    }

    func master(namespace: String) -> Data? {
        lock.withLock { try? Data(contentsOf: masterURL(namespace)) }
    }

    func wallet(namespace: String, recordId: String) -> Data? {
        lock.withLock { try? Data(contentsOf: walletURL(namespace, recordId)) }
    }

    func namespaces() -> [String] {
        lock.withLock { names(in: root.appendingPathComponent("masters")) }
    }

    func walletRecordIds(namespace: String) -> [String] {
        lock.withLock {
            names(in: root.appendingPathComponent("wallets").appendingPathComponent(safe(namespace)))
        }
    }

    func deleteWallet(namespace: String, recordId: String) {
        lock.withLock { try? FileManager.default.removeItem(at: walletURL(namespace, recordId)) }
    }

    func deleteNamespace(_ namespace: String) {
        lock.withLock {
            try? FileManager.default.removeItem(at: masterURL(namespace))
            try? FileManager.default.removeItem(
                at: root.appendingPathComponent("wallets").appendingPathComponent(safe(namespace))
            )
        }
    }

    private func masterURL(_ namespace: String) -> URL {
        root.appendingPathComponent("masters").appendingPathComponent(safe(namespace))
    }

    private func walletURL(_ namespace: String, _ recordId: String) -> URL {
        root.appendingPathComponent("wallets")
            .appendingPathComponent(safe(namespace))
            .appendingPathComponent(safe(recordId))
    }

    private func names(in directory: URL) -> [String] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []

        return names.filter { !$0.hasPrefix(".") }.sorted()
    }

    private func write(_ data: Data, to url: URL) throws {
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try CloudBackupUITestControl.writeAtomically(data, to: url)
    }

    /// namespaces and record ids are hex, so this only guards against a path separator
    private func safe(_ component: String) -> String {
        guard component.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }) else {
            fatalError("unexpected cloud backup path component \(component)")
        }

        return component
    }
}
