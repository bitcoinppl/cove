@testable import Cove
import CoveCore
import SwiftUI
import XCTest

/// How leaving the foreground covers, locks and dismisses, and how an NFC scan, whose system sheet
/// makes the app inactive, holds all three off
@MainActor
final class ScenePhasePrivacyTests: XCTestCase {
    private static let leavingActive: [ScenePhase] = [.inactive, .background]

    func testNfcScanKeepsTheAppUncoveredUnlockedAndItsSheetsUp() {
        for context in contexts(nfc: true) {
            let outcome = ScenePhasePrivacyOutcome(context)

            XCTAssertFalse(outcome.showsCover, "\(context)")
            XCTAssertFalse(outcome.dismissesSheets, "\(context)")

            // with auth off the lock is Rust's to skip, since the cached auth type can be stale
            XCTAssertEqual(outcome.locks, !context.isAuthEnabled && context.newPhase == .background, "\(context)")
        }
    }

    func testLeavingTheForegroundWithoutNfcCoversLocksAndDismisses() {
        for context in contexts(nfc: false) {
            let outcome = ScenePhasePrivacyOutcome(context)
            let backgrounds = context.newPhase == .background

            XCTAssertEqual(outcome.showsCover, context.oldPhase == .active || context.isAuthEnabled && backgrounds, "\(context)")
            XCTAssertEqual(outcome.locks, backgrounds, "\(context)")
            XCTAssertEqual(outcome.dismissesSheets, backgrounds && context.isAuthEnabled, "\(context)")
        }
    }

    /// A brief trip to inactive with a PIN clears its cover once the app is back, and going on to
    /// the background stops that, NFC scan or not
    func testCoverRecoveryFollowsAPinLockThroughInactive() {
        for context in contexts(nfc: true) + contexts(nfc: false) {
            let expected: ScenePhasePrivacyOutcome.CoverRecovery =
                switch (context.newPhase, context.isAuthEnabled) {
                case (.inactive, true) where !context.isUsingBiometrics: .restarted
                case (.background, true): .cancelled
                default: .unchanged
                }

            XCTAssertEqual(ScenePhasePrivacyOutcome(context).coverRecovery, expected, "\(context)")
        }
    }

    func testReturningToTheForegroundLeavesTheCoverAndLockToTheActivePhase() {
        for oldPhase in Self.leavingActive {
            for flags in authAndNfcFlags() {
                let context = ScenePhasePrivacyContext(
                    oldPhase: oldPhase,
                    newPhase: .active,
                    isAuthEnabled: flags.auth,
                    isUsingBiometrics: flags.biometrics,
                    isNfcOperationActive: flags.nfc
                )

                XCTAssertEqual(ScenePhasePrivacyOutcome(context), ScenePhasePrivacyOutcome(), "\(context)")
            }
        }
    }

    /// A tag read or write counts as an active operation whoever owns its reader or writer, such
    /// as a screen holding its own, and one that is released stops counting
    func testEveryNfcReaderAndWriterCountsAsAnActiveOperation() async throws {
        try await bootstrapIfNeeded()
        let app = AppManager.shared
        XCTAssertFalse(app.isNfcOperationActive, "the test host should hold no NFC scan")

        app.nfcReader.isScanning = true
        XCTAssertTrue(app.isNfcOperationActive, "the app's own tag read is an NFC operation")
        app.nfcReader.isScanning = false

        let screenReader = NFCReader()
        screenReader.isScanning = true
        XCTAssertTrue(app.isNfcOperationActive, "a screen's own tag read is an NFC operation")
        screenReader.isScanning = false
        XCTAssertFalse(app.isNfcOperationActive)

        let screenWriter = NFCWriter()
        screenWriter.isScanning = true
        XCTAssertTrue(app.isNfcOperationActive, "a tag write is an NFC operation")
        screenWriter.isScanning = false
        XCTAssertFalse(app.isNfcOperationActive)

        // a reader released mid-scan must not hold the cover and the lock off for good
        var releasedReader: NFCReader? = NFCReader()
        releasedReader?.isScanning = true
        XCTAssertTrue(app.isNfcOperationActive)
        releasedReader = nil
        XCTAssertFalse(app.isNfcOperationActive, "a released reader still counted as scanning")
    }

    /// A TAPSIGNER operation counts as an active operation from when it starts until it's
    /// cancelled
    ///
    /// The debug TAPSIGNER bridge points at a listener that never answers, so the operation stays
    /// open on its own until the test cancels it, with no NFC session for the simulator to end
    func testOpenTapSignerOperationCountsUntilCancelled() async throws {
        try await bootstrapIfNeeded()
        let app = AppManager.shared
        XCTAssertFalse(app.isNfcOperationActive, "the test host should hold no NFC scan")

        let listener = try SilentListener()
        defer { listener.close() }

        // the test host shares its environment with later tests, so put back whatever was there
        let key = UITestTapSignerBridge.environmentKey
        let previousBridge = ProcessInfo.processInfo.environment[key]
        setenv(key, "127.0.0.1:\(listener.port)", 1)
        defer {
            if let previousBridge {
                setenv(key, previousBridge, 1)
            } else {
                unsetenv(key)
            }
        }

        let tapSigner = TapSignerNFC(tapSignerPreviewNew(preview: true))
        let deriveReturned = expectation(description: "the cancelled derive returns")
        let derive = Task {
            let result = await tapSigner.derive(pin: "123456")
            deriveReturned.fulfill()
            return result
        }

        let deadline = Date().addingTimeInterval(10)
        while !app.isNfcOperationActive, Date() < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }

        XCTAssertTrue(app.isNfcOperationActive, "a started TAPSIGNER operation is an NFC operation")

        // the bridge never answers, so the operation is still open a second later
        try await Task.sleep(for: .seconds(1))
        XCTAssertTrue(app.isNfcOperationActive, "an open TAPSIGNER operation stopped counting")

        // cancel ends the scan before it returns, so a cancel that does nothing can't pass by
        // waiting out the bridge's receive timeout
        tapSigner.cancel()
        XCTAssertFalse(app.isNfcOperationActive, "a cancelled TAPSIGNER operation still counted")

        // well under the bridge's 30 second receive timeout, so only the cancel can end it in time
        await fulfillment(of: [deriveReturned], timeout: 5)
        let result = await derive.value

        guard case let .failure(.Unknown(message)) = result else {
            return XCTFail("the cancelled derive should fail as cancelled, got \(result)")
        }

        XCTAssertEqual(message, "TapSigner operation cancelled")
    }

    // MARK: - Helpers

    /// Every way out of the foreground: straight from active, and on from inactive to background,
    /// with auth and biometrics on and off
    private func contexts(nfc: Bool) -> [ScenePhasePrivacyContext] {
        let transitions: [(ScenePhase, ScenePhase)] = [
            (.active, .inactive),
            (.active, .background),
            (.inactive, .background),
        ]

        return transitions.flatMap { oldPhase, newPhase in
            authAndNfcFlags().map { flags in
                ScenePhasePrivacyContext(
                    oldPhase: oldPhase,
                    newPhase: newPhase,
                    isAuthEnabled: flags.auth,
                    isUsingBiometrics: flags.biometrics,
                    isNfcOperationActive: nfc
                )
            }
        }
    }

    private func authAndNfcFlags() -> [(auth: Bool, biometrics: Bool, nfc: Bool)] {
        [false, true].flatMap { auth in
            [false, true].flatMap { biometrics in
                [false, true].map { nfc in (auth, biometrics, nfc) }
            }
        }
    }

    private func bootstrapIfNeeded() async throws {
        do {
            _ = try await bootstrap()
        } catch AppInitError.AlreadyCalled(_) {}

        // the test host app bootstraps on its own, so AlreadyCalled can return before it finishes
        let deadline = Date().addingTimeInterval(30)
        while bootstrapProgress() != .complete {
            try XCTUnwrap(Date() < deadline ? true : nil, "bootstrap did not complete")
            try await Task.sleep(for: .milliseconds(50))
        }
    }
}

/// A TCP listener on the loopback interface that never accepts, so a client's connect succeeds
/// and its reads wait until the listener closes
private final class SilentListener {
    let port: UInt16
    private var fd: Int32

    init() throws {
        let socketFd = socket(AF_INET, SOCK_STREAM, 0)
        guard socketFd >= 0 else { throw POSIXError(.init(rawValue: errno) ?? .EIO) }

        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        address.sin_port = 0

        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let bound = withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { pointer in
                bind(socketFd, pointer, length) == 0
                    && listen(socketFd, 1) == 0
                    && getsockname(socketFd, pointer, &length) == 0
            }
        }

        guard bound else {
            let error = errno
            Darwin.close(socketFd)
            throw POSIXError(.init(rawValue: error) ?? .EIO)
        }

        fd = socketFd
        port = UInt16(bigEndian: address.sin_port)
    }

    deinit {
        close()
    }

    func close() {
        guard fd >= 0 else { return }

        Darwin.close(fd)
        fd = -1
    }
}
