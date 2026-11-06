import XCTest

/// The test's side of the scripted host's control directory
///
/// Each instance makes its own directory under `/tmp`, which the simulator shares with the host.
/// The test writes the script before launching, may rewrite it while the app runs, waits for the
/// host to enter a hold, and releases it. The host keeps its keychain and uploaded backups in the
/// same directory, so a relaunch with the same control finds them
final class CloudBackupHostControl {
    let directory: URL
    private(set) var script: CloudBackupUITestScript

    init(script: CloudBackupUITestScript) throws {
        directory = URL(fileURLWithPath: "/tmp/cove-cloud-ui-\(UUID().uuidString)", isDirectory: true)
        self.script = script
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try writeScript()
        Self.active.append(self)
    }

    /// Controls made during the current test, kept until `tearDown` so a running host never loses
    /// its directory
    private static var active: [CloudBackupHostControl] = []

    /// Stops the app, attaches its log to a failed test, and removes the test's control directories
    ///
    /// The host keeps writing `state.json` from background work after the test body returns, and
    /// treats a missing directory as a broken test, so the app stops before its directory goes.
    /// The log is attached here because the simulator clone is deleted after the test
    static func tearDown(_ testCase: XCTestCase, app: XCUIApplication) {
        app.terminate()

        for control in active {
            let log = control.directory.appendingPathComponent(CloudBackupUITestControl.appLogFileName)
            if testCase.testRun?.hasSucceeded == false, let text = try? String(contentsOf: log, encoding: .utf8) {
                let attachment = XCTAttachment(string: text)
                attachment.name = "app log"
                attachment.lifetime = .keepAlways
                testCase.add(attachment)
            }

            try? FileManager.default.removeItem(at: control.directory)
        }

        active.removeAll()
    }

    func configure(_ app: XCUIApplication) {
        app.launchEnvironment[CloudBackupUITestControl.environmentKey] = directory.path
    }

    /// Rewrites the script the running host reads on its next provider call
    func update(_ change: (inout CloudBackupUITestScript) -> Void) throws {
        change(&script)
        try writeScript()
    }

    /// Holds a later download of `recordId`: the first `skippingCalls` downloads after this call
    /// pass, and the next one waits for `release(id)`
    ///
    /// Counting from now keeps earlier inventory refreshes, which also read each record, from
    /// shifting which download is held
    func holdWalletDownload(id: String, recordId: String, skippingCalls: Int = 0) throws {
        let downloadsSoFar = state.count(.walletDownload(recordId: recordId))
        try update {
            $0.holds.append(.init(
                id: id,
                point: .walletDownload,
                recordId: recordId,
                matchingCallsToSkip: downloadsSoFar + skippingCalls
            ))
        }
    }

    /// Holds the next wallet upload made after this call, from whichever action makes it
    func holdNextWalletUpload(id: String) throws {
        let uploadsSoFar = state.count(.walletUpload)
        try update {
            $0.holds.append(.init(id: id, point: .walletUpload, matchingCallsToSkip: uploadsSoFar))
        }
    }

    /// Answers the next passkey assertions with `results`, whether the app authenticates with a
    /// known credential or discovers one; the last result repeats
    ///
    /// The host answers each call by its index across the test, so the calls made so far are
    /// padded with successes they already got
    func answerNextPasskeyAssertions(_ results: [CloudBackupUITestScript.PasskeyResult]) throws {
        let current = state
        try update {
            $0.authenticateResults = Array(repeating: .success, count: current.count(.passkeyAuthenticate)) + results
            $0.discoverResults = Array(repeating: .success, count: current.count(.passkeyDiscover)) + results
        }
    }

    /// Namespaces the cloud holds a master key for, read from the host's cloud directory
    var cloudNamespaces: [String] {
        cloudNames(in: "masters")
    }

    /// Wallet records the cloud holds under `namespace`
    func cloudWalletRecordIds(namespace: String) -> [String] {
        cloudNames(in: "wallets/\(namespace)")
    }

    private func cloudNames(in path: String) -> [String] {
        let url = directory
            .appendingPathComponent(CloudBackupUITestControl.cloudDirectoryName)
            .appendingPathComponent(path)
        let names = (try? FileManager.default.contentsOfDirectory(atPath: url.path)) ?? []

        return names.filter { !$0.hasPrefix(".") }.sorted()
    }

    func release(_ id: String) {
        let url = directory.appendingPathComponent(CloudBackupUITestControl.releaseFileName(id))
        FileManager.default.createFile(atPath: url.path, contents: Data())
    }

    func releaseVisibility() {
        release(CloudBackupUITestControl.visibilityGate)
    }

    /// Drops or restores the scripted network, and waits until the host has told Rust
    ///
    /// The provider follows the script on its next call, so only Rust's view needs the wait
    func setNetworkConnected(
        _ isConnected: Bool,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        try update { $0.networkConnected = isConnected }
        waitUntil(
            "the host never reported the network \(isConnected ? "up" : "down")",
            timeout: 10,
            file: file,
            line: line
        ) { $0.reportedNetworkConnected == isConnected }
    }

    /// The cloud backup manager's state as the app last wrote it, nil before it first did
    var appState: CloudBackupUITestAppState? {
        let url = directory.appendingPathComponent(CloudBackupUITestControl.appStateFileName)
        guard let data = try? Data(contentsOf: url) else { return nil }

        return try? JSONDecoder().decode(CloudBackupUITestAppState.self, from: data)
    }

    /// Waits until the app's cloud backup state satisfies `condition`
    func waitForAppState(
        _ message: String,
        timeout: TimeInterval = 30,
        file: StaticString = #filePath,
        line: UInt = #line,
        _ condition: (CloudBackupUITestAppState) -> Bool
    ) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let appState, condition(appState) { return }

            Thread.sleep(forTimeInterval: 0.1)
        }

        let last = appState
        XCTAssertTrue(
            last.map(condition) ?? false,
            "\(message); app state \(String(describing: last)); host state \(state)",
            file: file,
            line: line
        )
    }

    var state: CloudBackupUITestHostState {
        let url = directory.appendingPathComponent(CloudBackupUITestControl.stateFileName)
        guard let data = try? Data(contentsOf: url),
              let state = try? JSONDecoder().decode(CloudBackupUITestHostState.self, from: data)
        else { return CloudBackupUITestHostState() }

        return state
    }

    /// Waits until the host is parked on the hold named `id`
    func waitForHold(_ id: String, timeout: TimeInterval = 30, file: StaticString = #filePath, line: UInt = #line) {
        waitUntil("the host never entered hold \(id)", timeout: timeout, file: file, line: line) {
            $0.enteredHolds.contains(id)
        }
    }

    func waitUntil(
        _ message: String,
        timeout: TimeInterval = 30,
        file: StaticString = #filePath,
        line: UInt = #line,
        _ condition: (CloudBackupUITestHostState) -> Bool
    ) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if condition(state) { return }

            Thread.sleep(forTimeInterval: 0.1)
        }

        XCTAssertTrue(condition(state), "\(message); host state \(state)", file: file, line: line)
    }

    /// Whether the host's persisted keychain holds an item under `key`, throwing when the keychain
    /// can't be read so an unreadable file never reads as a removed item
    func keychainContains(_ key: String) throws -> Bool {
        let url = directory.appendingPathComponent(CloudBackupUITestControl.keychainFileName)
        let values = try JSONDecoder().decode([String: String].self, from: Data(contentsOf: url))

        return values[key] != nil
    }

    /// Waits until the host's persisted keychain no longer holds an item under `key`
    func waitForKeychainItemRemoved(
        _ key: String,
        _ message: String,
        timeout: TimeInterval = 30,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let deadline = Date().addingTimeInterval(timeout)
        while try keychainContains(key), Date() < deadline {
            Thread.sleep(forTimeInterval: 0.1)
        }

        XCTAssertFalse(try keychainContains(key), message, file: file, line: line)
    }

    /// Removes keychain items whose keys contain `fragment`, as if the keychain lost them while the
    /// app was not running
    func removeKeychainItems(containing fragment: String) throws -> Int {
        let url = directory.appendingPathComponent(CloudBackupUITestControl.keychainFileName)
        var values = try JSONDecoder().decode([String: String].self, from: Data(contentsOf: url))
        let before = values.count
        values = values.filter { !$0.key.contains(fragment) }
        try CloudBackupUITestControl.writeAtomically(JSONEncoder().encode(values), to: url)

        return before - values.count
    }

    private func writeScript() throws {
        let data = try JSONEncoder().encode(script)
        try CloudBackupUITestControl.writeAtomically(
            data,
            to: directory.appendingPathComponent(CloudBackupUITestControl.scriptFileName)
        )
    }
}
