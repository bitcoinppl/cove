import AuthenticationServices
@testable import Cove
import CoveCore
import XCTest

final class PasskeyProviderImplTests: XCTestCase {
    func testRequestModesDescribeEachNativeOperation() {
        XCTAssertEqual(PasskeyOperationContext.registration.requestMode, .registration)
        XCTAssertEqual(PasskeyOperationContext.discoverAssertion.requestMode, .discovery)
        XCTAssertEqual(PasskeyOperationContext.authenticateAssertion.requestMode, .targeted)

        let presence = PasskeyRequestDiagnostics(
            rpId: "example.com",
            operation: PasskeyRequestMode.presence.rawValue,
            requestMode: .presence
        )
        XCTAssertEqual(presence.requestMode, .presence)
    }

    func testDiagnosticsMeasureMonotonicRequestDurationsAndAnchorState() {
        let diagnostics = PasskeyRequestDiagnostics(
            rpId: "example.com",
            operation: "targeted",
            requestMode: .targeted
        )
        let submission = ContinuousClock.Instant.now
        let anchor = submission.advanced(by: .milliseconds(12))
        let completion = submission.advanced(by: .milliseconds(34))

        diagnostics.markNativeSubmission(at: submission)
        diagnostics.markPresentationAnchorRequest(
            at: anchor,
            isAvailable: true,
            sceneActivation: "foregroundActive"
        )
        diagnostics.markCompletion(at: completion)

        let fields = diagnostics.logFields()
        XCTAssertTrue(fields.contains("request_id="))
        XCTAssertEqual(fields.components(separatedBy: "rpId=").count, 2)
        XCTAssertTrue(fields.contains("rpId=example.com"))
        XCTAssertTrue(fields.contains("operation=targeted"))
        XCTAssertTrue(fields.contains("request_mode=targeted"))
        XCTAssertTrue(fields.contains("submission_to_anchor_ms=12"))
        XCTAssertTrue(fields.contains("submission_to_completion_ms=34"))
        XCTAssertTrue(fields.contains("presentation_anchor_requested=true"))
        XCTAssertTrue(fields.contains("presentation_anchor_available=true"))
        XCTAssertTrue(fields.contains("presentation_scene_activation=foregroundActive"))
    }

    func testNSErrorMetadataContainsOnlyDomainAndCode() {
        let error = NSError(
            domain: "com.example.passkey",
            code: 42,
            userInfo: [
                NSLocalizedDescriptionKey: "private localized description",
                "private": "private user info",
            ]
        )

        let metadata = passkeyNSErrorMetadata(error)

        XCTAssertEqual(
            metadata,
            "error_domain=com.example.passkey error_code=42"
        )
        XCTAssertFalse(metadata.contains("private"))
        XCTAssertFalse(metadata.contains("localized"))
        XCTAssertFalse(metadata.contains("userInfo"))
    }

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
        let terminalDiagnostics = delegate.diagnostics.logFields()

        delegate.authorizationController(
            controller: ASAuthorizationController(authorizationRequests: [request]),
            didCompleteWithError: NSError(
                domain: "PasskeyProviderImplTests",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: "second late callback"]
            )
        )

        XCTAssertEqual(delegate.diagnostics.logFields(), terminalDiagnostics)

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
