import AuthenticationServices
@testable import Cove
import CoveCore
import XCTest

final class PasskeyProviderImplTests: XCTestCase {
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
}
