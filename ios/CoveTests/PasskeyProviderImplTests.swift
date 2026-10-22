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

    func testNonAuthorizationFailureReturnsOnlySanitizedMetadata() {
        let delegate = PasskeyDelegate(context: .discoverAssertion)
        let request = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: "example.com"
        ).createCredentialAssertionRequest(challenge: Data(count: 32))

        delegate.authorizationController(
            controller: ASAuthorizationController(authorizationRequests: [request]),
            didCompleteWithError: NSError(
                domain: "com.example.passkey",
                code: 42,
                userInfo: [NSLocalizedDescriptionKey: "private localized description"]
            )
        )

        XCTAssertThrowsError(try delegate.waitForResult {}) { error in
            let description = String(describing: error)

            XCTAssertTrue(description.contains("com.example.passkey"))
            XCTAssertTrue(description.contains("42"))
            XCTAssertFalse(description.contains("private"))
            XCTAssertFalse(description.contains("localized"))
        }
    }

    func testInteractiveRequestTimeoutCancelsOnMainQueueAndReturnsPresentedFailure() {
        let delegate = PasskeyDelegate(context: .registration, timeout: 0.01)
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
        let delegate = PasskeyDelegate(context: .authenticateAssertion, timeout: 0.01)
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

        guard case .userCancelled = failure else {
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
