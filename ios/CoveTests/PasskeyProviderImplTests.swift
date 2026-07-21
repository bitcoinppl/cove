import AuthenticationServices
@testable import Cove
import CoveCore
import XCTest

final class PasskeyProviderImplTests: XCTestCase {
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

        wait(for: [cancellation], timeout: 1)
    }

    func testLateCallbackCannotReplaceTimeoutResult() {
        let delegate = PasskeyDelegate(context: .authenticateAssertion, timeout: 0.01)
        let cancellation = expectation(description: "controller cancelled")

        XCTAssertThrowsError(
            try delegate.waitForResult {
                cancellation.fulfill()
            }
        )
        wait(for: [cancellation], timeout: 1)

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
