import AuthenticationServices
@testable import Cove
import CoveCore
import XCTest

final class PasskeyProviderImplTests: XCTestCase {
    /// Upper bound for a timed-out request's cancellation, which runs on the main queue
    ///
    /// The host app can keep the main queue busy for seconds on a slow CI simulator; the wait still
    /// returns as soon as cancellation runs
    private static let mainQueueCancellationTimeout: TimeInterval = 10

    private let windows = StubWindows()

    override func tearDown() {
        MainActor.assumeIsolated {
            windows.close()
        }

        super.tearDown()
    }

    /// XCTest runs these synchronous tests on the main thread, where UIKit objects may be created
    private static func anchor() -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            ASPresentationAnchor()
        }
    }

    func testRequestsDoNotStartWhenPresenterIsNotReady() {
        let starts = NativeStarts()
        let provider = PasskeyProviderImpl(
            waitForPresenter: { .notReady },
            startRequest: starts.record
        )

        runEachRequest(provider) { error, expectedOperation in
            guard case let PasskeyError.NotReady(operation) = error else { return false }
            return operation == expectedOperation
        }

        XCTAssertEqual(starts.count, 0)
    }

    /// Rust can revoke the permit after the presenter settled, while the request waits for the main
    /// queue, so the main-queue turn that would start the request asks Rust again
    func testRequestsDoNotStartWhenTheirOperationEndsWhileWaitingForTheMainQueue() {
        let window = MainActor.assumeIsolated { UIWindow() }
        let permit = Flag(true)
        let starts = NativeStarts()
        let provider = PasskeyProviderImpl(
            waitForPresenter: {
                permit.value = true
                Self.holdMainQueue { permit.value = false }
                return .ready(window)
            },
            presenterIsStillReady: { _ in true },
            requestMayPresent: { permit.value },
            startRequest: starts.record
        )

        runEachRequest(provider) { error, _ in
            guard case PasskeyError.NotCompleted = error else { return false }
            return true
        }

        XCTAssertEqual(starts.count, 0)
    }

    /// The app can lock after the presenter settled, while the request waits for the main queue.
    /// The lock is a SwiftUI overlay that leaves the window active and idle, so only the lock state
    /// keeps the request from starting over it
    func testRequestsDoNotStartWhenTheAppLocksWhileWaitingForTheMainQueue() throws {
        let window = try MainActor.assumeIsolated { try windows.make() }
        let appLock = Flag(false)
        let starts = NativeStarts()
        let provider = PasskeyProviderImpl(
            waitForPresenter: {
                appLock.value = false
                Self.holdMainQueue { appLock.value = true }
                return .ready(window)
            },
            presenterIsStillReady: { anchor in
                PresenterIdle.isStillReady(
                    anchor,
                    scenes: { [StubScene(activationState: .foregroundActive, windows: [window])] },
                    isAppLocked: { appLock.value }
                )
            },
            requestMayPresent: { true },
            startRequest: starts.record
        )

        runEachRequest(provider) { error, expectedOperation in
            guard case let PasskeyError.NotReady(operation) = error else { return false }
            return operation == expectedOperation
        }

        XCTAssertEqual(starts.count, 0)
    }

    /// The control for the two tests above: the same main-queue wait with the presenter still ready,
    /// the app unlocked and the permit live starts every request
    func testRequestsStartWhenThePresenterStaysReadyAndThePermitLive() throws {
        let window = try MainActor.assumeIsolated { try windows.make() }
        let starts = NativeStarts()
        let provider = PasskeyProviderImpl(
            waitForPresenter: {
                Self.holdMainQueue {}
                return .ready(window)
            },
            presenterIsStillReady: { anchor in
                PresenterIdle.isStillReady(
                    anchor,
                    scenes: { [StubScene(activationState: .foregroundActive, windows: [window])] },
                    isAppLocked: { false }
                )
            },
            requestMayPresent: { true },
            startRequest: starts.record
        )

        runEachRequest(provider) { error, expectedOperation in
            guard case let PasskeyError.RequestFailed(operation, _) = error else { return false }
            return operation == expectedOperation
        }

        XCTAssertEqual(starts.count, 3)
    }

    /// Occupies the main queue, then runs `release` on it before letting the queue move on
    ///
    /// Returns once the main queue is held, so the request's main-queue turn queues behind this one
    /// and runs only after `release`
    private static func holdMainQueue(thenOnMain release: @escaping @Sendable () -> Void) {
        let held = DispatchSemaphore(value: 0)
        DispatchQueue.main.async {
            held.signal()

            // long enough for the request to pass its presenter wait and queue its main-queue turn
            Thread.sleep(forTimeInterval: 0.2)
            release()
        }

        held.wait()
    }

    /// Runs each interactive request and expects each to throw an error `matches` accepts
    private func runEachRequest(
        _ provider: PasskeyProviderImpl,
        matches: @escaping @Sendable (Error, PasskeyOperation) -> Bool
    ) {
        let finished = expectation(description: "requests returned")
        let requests: [(PasskeyOperation, () throws -> Void)] = [
            (.registration, {
                _ = try provider.createPasskey(
                    rpId: "example.com",
                    challenge: Data(count: 32),
                    user: PasskeyRegistrationUser(id: Data([1]), name: "test", displayName: "Test")
                )
            }),
            (.authenticateAssertion, {
                _ = try provider.authenticateWithPrf(
                    rpId: "example.com",
                    credentialId: Data([1]),
                    prfSalt: Data(count: 32),
                    challenge: Data(count: 32)
                )
            }),
            (.discoverAssertion, {
                _ = try provider.discoverAndAuthenticateWithPrf(
                    rpId: "example.com",
                    prfSalt: Data(count: 32),
                    challenge: Data(count: 32)
                )
            }),
        ]

        // the provider blocks its caller, so run it off the main thread like Rust does
        DispatchQueue.global().async {
            for (expectedOperation, request) in requests {
                do {
                    try request()
                    XCTFail("expected \(expectedOperation) to throw")
                } catch {
                    if !matches(error, expectedOperation) {
                        XCTFail("unexpected failure for \(expectedOperation): \(error)")
                    }
                }
            }

            finished.fulfill()
        }

        wait(for: [finished], timeout: Self.mainQueueCancellationTimeout)
    }

    /// The domain and code travel to Rust for its typed diagnostic. The error's description
    /// (Rust's redacted `Display`) and the delegate's log line both come from Rust's one allowlist,
    /// so an allowlisted domain keeps its code and any other domain, its code and localized text
    /// never show
    func testNonAuthorizationFailureLogsOnlyTheRedactedForm() {
        let cases: [(domain: String, code: Int, logged: String)] = [
            ("NSCocoaErrorDomain", 4097, "failed with non-auth error error_domain=NSCocoaErrorDomain error_code=4097"),
            ("com.example.passkey", 40442, "failed with non-auth error error_domain=unlisted"),
        ]

        for (domain, code, logged) in cases {
            let delegate = PasskeyDelegate(context: .discoverAssertion, anchor: Self.anchor())
            let request = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: "example.com"
            ).createCredentialAssertionRequest(challenge: Data(count: 32))
            let logsBefore = SwiftLogStore.shared.snapshot()

            delegate.authorizationController(
                controller: ASAuthorizationController(authorizationRequests: [request]),
                didCompleteWithError: NSError(
                    domain: domain,
                    code: code,
                    userInfo: [NSLocalizedDescriptionKey: "private localized description"]
                )
            )

            XCTAssertThrowsError(try delegate.waitForResult {}) { error in
                guard case let PasskeyError.RequestFailed(_, reason) = error,
                      case let .unknown(diagnosticMessage) = reason
                else {
                    XCTFail("expected an unknown request failure")
                    return
                }

                XCTAssertEqual(diagnosticMessage, "error_domain=\(domain) error_code=\(code)")

                let description = String(describing: error)
                XCTAssertFalse(description.contains("private"), description)
                XCTAssertFalse(description.contains("com.example.passkey"), description)
                XCTAssertFalse(description.contains("error_code=40442"), description)
            }

            let logsAfter = SwiftLogStore.shared.snapshot()
            let written = logsAfter.hasPrefix(logsBefore) ? String(logsAfter.dropFirst(logsBefore.count)) : logsAfter
            XCTAssertTrue(written.contains(logged), written)
            XCTAssertFalse(written.contains("private"), written)
            XCTAssertFalse(written.contains("com.example.passkey"), written)
            XCTAssertFalse(written.contains("error_code=40442"), written)
        }
    }

    func testInteractiveRequestTimeoutCancelsOnMainQueueAndReturnsPresentedFailure() {
        let delegate = PasskeyDelegate(context: .registration, anchor: Self.anchor(), timeout: 0.01)
        let cancellation = expectation(description: "controller cancelled")

        XCTAssertThrowsError(
            try delegate.waitForResult {
                XCTAssertTrue(Thread.isMainThread)
                cancellation.fulfill()
            }
        ) { error in
            guard case let PasskeyError.RequestFailed(operation, reason) = error,
                  operation == .registration,
                  case .platformAuthorizationFailedAfterPresentation = reason
            else {
                XCTFail("expected registration timeout to be a post-presentation platform failure")
                return
            }
        }

        wait(for: [cancellation], timeout: Self.mainQueueCancellationTimeout)
    }

    func testLateCallbackCannotReplaceTimeoutResult() {
        let delegate = PasskeyDelegate(context: .authenticateAssertion, anchor: Self.anchor(), timeout: 0.01)
        let cancellation = expectation(description: "controller cancelled")

        XCTAssertThrowsError(
            try delegate.waitForResult {
                cancellation.fulfill()
            }
        )
        wait(for: [cancellation], timeout: Self.mainQueueCancellationTimeout)

        let request = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: "example.com"
        ).createCredentialAssertionRequest(challenge: Data(count: 32))

        delegate.authorizationController(
            controller: ASAuthorizationController(authorizationRequests: [request]),
            didCompleteWithError: NSError(
                domain: "PasskeyProviderImplTests",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "late callback"]
            )
        )

        XCTAssertThrowsError(
            try delegate.waitForResult {
                XCTFail("terminal timeout must not schedule cancellation twice")
            }
        ) { error in
            guard case let PasskeyError.RequestFailed(operation, reason) = error,
                  operation == .authenticateAssertion,
                  case .platformAuthorizationFailedAfterPresentation = reason
            else {
                XCTFail("expected the original timeout result")
                return
            }
        }
    }

    func testFailedBeforePresentationIsRetryablePlatformFailure() {
        let failure = passkeyAuthorizationFailure(
            for: .failed,
            didRequestPresentationAnchor: false,
            diagnosticMessage: "not associated with domain"
        )

        guard case let .requestFailed(reason) = failure,
              case .platformAuthorizationFailed = reason
        else {
            XCTFail("expected pre-presentation platform authorization failure")
            return
        }
    }

    func testFailedAfterPresentationPreservesPlatformFailureSemantics() {
        let failure = passkeyAuthorizationFailure(
            for: .failed,
            didRequestPresentationAnchor: true,
            diagnosticMessage: "not associated with domain"
        )

        guard case let .requestFailed(reason) = failure,
              case .platformAuthorizationFailedAfterPresentation = reason
        else {
            XCTFail("expected post-presentation platform authorization failure")
            return
        }
    }

    func testCancellationRemainsCancellation() {
        let failure = passkeyAuthorizationFailure(
            for: .canceled,
            didRequestPresentationAnchor: false,
            diagnosticMessage: "cancelled"
        )

        guard case .notCompleted = failure else {
            XCTFail("expected cancellation")
            return
        }
    }

    func testUnexpectedDiscoveryCredentialCannotAuthorizeRegistrationFallback() {
        let failure = passkeyUnexpectedCredentialError(operation: .discoverAssertion)

        guard case let .RequestFailed(operation, reason) = failure,
              operation == .discoverAssertion,
              case .unexpectedCredentialType = reason
        else {
            XCTFail("expected unexpected discovery credential type to remain a request failure")
            return
        }
    }
}

/// A flag the request thread and the main queue both read and write
private final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: Bool

    init(_ value: Bool) {
        stored = value
    }

    var value: Bool {
        get { lock.withLock { stored } }
        set { lock.withLock { stored = newValue } }
    }
}

/// Counts native request starts in place of `performRequests()`
///
/// Each start is answered at once with a failure, so the blocked request returns instead of
/// waiting out its five-minute bound
private final class NativeStarts: @unchecked Sendable {
    private let lock = NSLock()
    private var stored = 0

    var count: Int {
        lock.withLock { stored }
    }

    @MainActor
    func record(_ controller: ASAuthorizationController) {
        lock.withLock { stored += 1 }
        controller.delegate?.authorizationController?(
            controller: controller,
            didCompleteWithError: NSError(domain: "PasskeyProviderImplTests.started", code: 1)
        )
    }
}
