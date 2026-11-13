import AuthenticationServices
@_exported import CoveCore
import CryptoKit
import Foundation
import UIKit

enum PasskeyOperationContext: Equatable {
    case registration
    case discoverAssertion
    case authenticateAssertion

    var logDescription: String {
        switch self {
        case .registration:
            "registration"
        case .discoverAssertion:
            "discover assertion"
        case .authenticateAssertion:
            "authenticate assertion"
        }
    }

    var operation: PasskeyOperation {
        switch self {
        case .registration:
            .registration
        case .discoverAssertion:
            .discoverAssertion
        case .authenticateAssertion:
            .authenticateAssertion
        }
    }
}

/// Domain and code only, so localized text never reaches the error surfaced to Rust
///
/// Rust types it and logs only its redacted form; Swift logs use `passkeyErrorLogLine` instead
private func passkeyNSErrorMetadata(_ error: Error) -> String {
    let nsError = error as NSError
    return "error_domain=\(nsError.domain) error_code=\(nsError.code)"
}

/// The only form in which a Swift log shows the platform error behind a passkey request
///
/// Rust's one allowlist keeps a known domain with its code and hides any other domain and code
func passkeyErrorLogLine(_ error: Error) -> String {
    let nsError = error as NSError
    return passkeyRequestLogLine(domain: nsError.domain, code: Int64(nsError.code))
}

/// Drop the last references to a finished request on the main thread
///
/// The blocking caller otherwise releases the controller off the main thread while
/// AuthenticationServices is still unwinding its delegate callback for that controller
private func releasePasskeyRequestOnMain(
    _ controller: ASAuthorizationController,
    _ delegate: AnyObject
) {
    DispatchQueue.main.async {
        withExtendedLifetime((controller, delegate)) {}
    }
}

func passkeyUnexpectedCredentialError(
    operation: PasskeyOperation
) -> PasskeyError {
    PasskeyError.RequestFailed(
        operation: operation,
        reason: .unexpectedCredentialType
    )
}

final class PasskeyProviderImpl: PasskeyProvider, @unchecked Sendable {
    private let waitForPresenter: () -> PresenterIdle.Readiness
    private let presenterIsStillReady: @MainActor (UIWindow) -> Bool
    private let requestMayPresent: () -> Bool
    private let startRequest: @MainActor (ASAuthorizationController) -> Void

    init(
        waitForPresenter: @escaping () -> PresenterIdle.Readiness = PresenterIdle.waitUntilReadyBlocking,
        presenterIsStillReady: @escaping @MainActor (UIWindow) -> Bool = { PresenterIdle.isStillReady($0) },
        requestMayPresent: @escaping () -> Bool = passkeyRequestMayPresent,
        startRequest: @escaping @MainActor (ASAuthorizationController) -> Void = { $0.performRequests() }
    ) {
        self.waitForPresenter = waitForPresenter
        self.presenterIsStillReady = presenterIsStillReady
        self.requestMayPresent = requestMayPresent
        self.startRequest = startRequest
    }

    private enum RegistrationPrfSupportState {
        case confirmedSupported
        case unknown
    }

    private enum PrfExtractionError: Error {
        case outputUnavailable
        case outputTooShort(Int)

        var logDescription: String {
            switch self {
            case .outputUnavailable:
                "PRF output not available"
            case let .outputTooShort(count):
                "PRF output too short: \(count) bytes, need 32"
            }
        }
    }

    private func credentialSummary(_ credentialId: Data) -> String {
        let fingerprint = SHA256.hash(data: credentialId)
            .prefix(6)
            .map { String(format: "%02x", $0) }
            .joined()
        return "len=\(credentialId.count) fingerprint=\(fingerprint)"
    }

    /// PRF is guaranteed on iOS 18.4+ (our minimum deployment target)
    func isPrfSupported() -> Bool {
        true
    }

    /// Waits for a ready presenter and returns the window the request anchors to
    ///
    /// Throws instead of proceeding when none is ready, so a request never starts over a locked
    /// app, an inactive scene or a busy presenter
    private func readyPresentationAnchor(for context: PasskeyOperationContext) throws -> UIWindow {
        switch waitForPresenter() {
        case let .ready(window):
            window

        case .notReady:
            Log.warn("[PASSKEY] \(context.logDescription) not started: presenter not ready")
            throw PasskeyError.NotReady(operation: context.operation)
        }
    }

    /// Starts an interactive request in the main-queue turn that confirms it may still start
    ///
    /// The blocked caller waits for the main queue, with no bound, after its presenter settled.
    /// During that wait Rust can revoke the operation's permit, and the app can lock, leave the
    /// foreground or present an alert. So the presenter and then Rust's permit are checked here,
    /// with nothing between the last check and `performRequests()`
    @MainActor
    private func start(
        _ controller: ASAuthorizationController,
        anchoredTo anchor: UIWindow,
        for context: PasskeyOperationContext
    ) throws {
        guard presenterIsStillReady(anchor) else {
            Log.warn("[PASSKEY] \(context.logDescription) not started: presenter no longer ready")
            throw PasskeyError.NotReady(operation: context.operation)
        }

        guard requestMayPresent() else {
            Log.info("[PASSKEY] \(context.logDescription) not started: its operation ended")
            throw PasskeyError.NotCompleted
        }

        startRequest(controller)
    }

    func createPasskey(rpId: String, challenge: Data, user: PasskeyRegistrationUser) throws -> PasskeyRegistrationResult {
        precondition(!Thread.isMainThread, "createPasskey must not be called from the main thread")

        let registration = try performRegistrationRequest(
            rpId: rpId,
            challenge: challenge,
            user: user
        )
        _ = try validateRegistrationPrfMetadata(registration)

        let providerAaguid: String
        if let attestationObject = registration.rawAttestationObject {
            providerAaguid = try passkeyAaguidFromAttestationObject(
                attestationObject: attestationObject
            )
        } else {
            Log.warn("[PASSKEY] registration attestation object missing, using iOS fallback AAGUID")
            providerAaguid = "00000000-0000-0000-0000-000000000000"
        }

        return PasskeyRegistrationResult(
            credentialId: registration.credentialID,
            providerAaguid: providerAaguid,
            registeredPlatform: .ios
        )
    }

    func authenticateWithPrf(
        rpId: String, credentialId: Data, prfSalt: Data, challenge: Data
    ) throws -> Data {
        precondition(
            !Thread.isMainThread,
            "authenticateWithPrf must not be called from the main thread"
        )

        let (prfOutput, _) = try performPrfAssertion(
            rpId: rpId,
            credentialId: credentialId,
            prfSalt: prfSalt,
            challenge: challenge,
            context: .authenticateAssertion
        )
        return prfOutput
    }

    func checkPasskeyPresence(rpId: String, credentialId: Data) -> PasskeyCredentialPresence {
        precondition(
            !Thread.isMainThread,
            "checkPasskeyPresence must not be called from the main thread"
        )

        // wait before building the controller and before any timeout starts, so the wait never
        // eats the request timeout
        guard case let .ready(anchor) = waitForPresenter() else {
            Log.warn("[PASSKEY] presence check not started: presenter not ready")
            return .indeterminate
        }

        // passkey authorization requests can present iOS UI, so do not use this for background polling
        let credentialSummary = credentialSummary(credentialId)
        Log.info("[PASSKEY] presence check start rpId=\(rpId) credential=\(credentialSummary)")

        let delegate = PasskeyExistenceDelegate(anchor: anchor)
        let controller: ASAuthorizationController

        controller = DispatchQueue.main.sync {
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: rpId
            )

            let request = provider.createCredentialAssertionRequest(
                challenge: Data(count: 32)
            )

            request.allowedCredentials = [
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(
                    credentialID: credentialId
                ),
            ]

            let ctrl = ASAuthorizationController(authorizationRequests: [request])
            ctrl.delegate = delegate
            ctrl.presentationContextProvider = delegate
            ctrl.performRequests(options: .preferImmediatelyAvailableCredentials)
            return ctrl
        }
        defer { releasePasskeyRequestOnMain(controller, delegate) }

        // .notInteractive returns almost instantly when no credential exists.
        // if iOS doesn't respond quickly enough to prove presence or absence,
        // treat the result as indeterminate instead of assuming success.
        let gotResult = delegate.semaphore.wait(timeout: .now() + 1.0)

        if gotResult == .timedOut {
            Log.warn(
                "[PASSKEY] presence check timed out after 1s"
            )
            DispatchQueue.main.async { controller.cancel() }
            return .indeterminate
        }

        Log.info(
            "[PASSKEY] presence check resolved rpId=\(rpId) credential=\(credentialSummary) presence=\(delegate.presence)"
        )
        return delegate.presence
    }

    func discoverAndAuthenticateWithPrf(
        rpId: String, prfSalt: Data, challenge: Data
    ) throws -> DiscoveredPasskeyResult {
        precondition(
            !Thread.isMainThread,
            "discoverAndAuthenticateWithPrf must not be called from the main thread"
        )

        let (prfOutput, assertion) = try performPrfAssertion(
            rpId: rpId,
            credentialId: nil,
            prfSalt: prfSalt,
            challenge: challenge,
            context: .discoverAssertion
        )
        return DiscoveredPasskeyResult(
            prfOutput: prfOutput,
            credentialId: assertion.credentialID
        )
    }

    private func performRegistrationRequest(
        rpId: String,
        challenge: Data,
        user: PasskeyRegistrationUser
    ) throws -> ASAuthorizationPlatformPublicKeyCredentialRegistration {
        // wait before building the controller and before any timeout starts, so the wait never
        // eats the request timeout
        let anchor = try readyPresentationAnchor(for: .registration)

        let delegate = PasskeyDelegate(context: .registration, anchor: anchor)
        let controller: ASAuthorizationController

        controller = try DispatchQueue.main.sync {
            Log.info("[PASSKEY] registration request start rpId=\(rpId)")
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: rpId
            )

            let request = provider.createCredentialRegistrationRequest(
                challenge: challenge,
                name: user.name,
                userID: user.id
            )

            // keep registration and PRF assertions on the same verified-user policy
            request.userVerificationPreference = .required
            request.displayName = user.displayName
            request.prf = .checkForSupport

            let ctrl = ASAuthorizationController(authorizationRequests: [request])
            ctrl.delegate = delegate
            ctrl.presentationContextProvider = delegate
            try MainActor.assumeIsolated { try start(ctrl, anchoredTo: anchor, for: .registration) }
            return ctrl
        }
        defer { releasePasskeyRequestOnMain(controller, delegate) }

        let credential = try delegate.waitForResult {
            controller.cancel()
        }

        guard
            let registration =
            credential as? ASAuthorizationPlatformPublicKeyCredentialRegistration
        else {
            throw passkeyUnexpectedCredentialError(operation: .registration)
        }

        Log.info("[PASSKEY] registration request succeeded credential_len=\(registration.credentialID.count)")
        return registration
    }

    private func validateRegistrationPrfMetadata(
        _ registration: ASAuthorizationPlatformPublicKeyCredentialRegistration
    ) throws -> RegistrationPrfSupportState {
        guard let prfOutput = registration.prf else {
            Log.warn("[PASSKEY] registration PRF metadata is missing, deferring support check to assertion")
            return .unknown
        }

        Log.info("[PASSKEY] registration PRF supported: \(prfOutput.isSupported)")

        guard prfOutput.isSupported else {
            Log.warn("[PASSKEY] registration PRF is unsupported by this passkey provider")
            throw PasskeyError.PrfUnsupportedProvider
        }

        return .confirmedSupported
    }

    private func performPrfAssertion(
        rpId: String,
        credentialId: Data?,
        prfSalt: Data,
        challenge: Data,
        context: PasskeyOperationContext
    ) throws -> (Data, ASAuthorizationPlatformPublicKeyCredentialAssertion) {
        // avoid an automatic second assertion here because targeted auth retries
        // can cause the native sign-in sheet to disappear and reappear
        let assertion = try performAssertionRequest(
            rpId: rpId,
            credentialId: credentialId,
            prfSalt: prfSalt,
            challenge: challenge,
            context: context
        )

        do {
            let prfOutput = try extractPrfOutput(from: assertion, context: context)
            return (prfOutput, assertion)
        } catch let error as PrfExtractionError {
            Log.warn(
                "[PASSKEY] \(context.logDescription) could not obtain usable PRF output: \(error.logDescription)"
            )
            throw PasskeyError.PrfUnsupportedProvider
        }
    }

    private func performAssertionRequest(
        rpId: String,
        credentialId: Data?,
        prfSalt: Data,
        challenge: Data,
        context: PasskeyOperationContext
    ) throws -> ASAuthorizationPlatformPublicKeyCredentialAssertion {
        // wait before building the controller and before any timeout starts, so the wait never
        // eats the request timeout
        let anchor = try readyPresentationAnchor(for: context)

        let delegate = PasskeyDelegate(context: context, anchor: anchor)
        let controller: ASAuthorizationController

        controller = try DispatchQueue.main.sync {
            Log.info(
                "[PASSKEY] \(context.logDescription) request start rpId=\(rpId) targeted=\(credentialId != nil)"
            )
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: rpId
            )

            let request = provider.createCredentialAssertionRequest(
                challenge: challenge
            )

            if let credentialId {
                request.allowedCredentials = [
                    ASAuthorizationPlatformPublicKeyCredentialDescriptor(
                        credentialID: credentialId
                    ),
                ]
            } else {
                request.allowedCredentials = []
            }

            // PRF derives different secrets for verified and unverified requests
            request.userVerificationPreference = .required
            request.prf = .inputValues(.init(saltInput1: prfSalt))

            let ctrl = ASAuthorizationController(authorizationRequests: [request])
            ctrl.delegate = delegate
            ctrl.presentationContextProvider = delegate
            try MainActor.assumeIsolated { try start(ctrl, anchoredTo: anchor, for: context) }
            return ctrl
        }
        defer { releasePasskeyRequestOnMain(controller, delegate) }

        let credential = try delegate.waitForResult {
            controller.cancel()
        }

        guard
            let assertion =
            credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion
        else {
            throw passkeyUnexpectedCredentialError(operation: context.operation)
        }

        Log.info("[PASSKEY] \(context.logDescription) request succeeded credential_len=\(assertion.credentialID.count)")
        return assertion
    }

    private func extractPrfOutput(
        from assertion: ASAuthorizationPlatformPublicKeyCredentialAssertion,
        context: PasskeyOperationContext
    ) throws -> Data {
        if assertion.prf == nil {
            Log.error("[PASSKEY] \(context.logDescription) PRF output is missing")
        }

        guard let prfKey = assertion.prf?.first else {
            throw PrfExtractionError.outputUnavailable
        }

        let prfOutput = prfKey.withUnsafeBytes { Data($0) }

        guard prfOutput.count >= 32 else {
            throw PrfExtractionError.outputTooShort(prfOutput.count)
        }

        return prfOutput.prefix(32)
    }
}

// MARK: - PasskeyDelegate

final class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    static let interactiveRequestTimeout: TimeInterval = 5 * 60

    private let semaphore = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private let timeout: TimeInterval
    private var result: Result<ASAuthorizationCredential, Error>?
    private let context: PasskeyOperationContext
    private let anchor: ASPresentationAnchor
    private var didRequestPresentationAnchor = false

    init(
        context: PasskeyOperationContext,
        anchor: ASPresentationAnchor,
        timeout: TimeInterval = PasskeyDelegate.interactiveRequestTimeout
    ) {
        self.context = context
        self.anchor = anchor
        self.timeout = timeout
    }

    func waitForResult(
        cancelController: @escaping @Sendable () -> Void
    ) throws -> ASAuthorizationCredential {
        let waitResult = semaphore.wait(timeout: .now() + timeout)

        if waitResult == .timedOut {
            let timeoutError = PasskeyError.RequestFailed(
                operation: context.operation,
                reason: .platformAuthorizationFailedAfterPresentation
            )

            if complete(with: .failure(timeoutError)) {
                Log.warn(
                    "[PASSKEY] \(context.logDescription) timed out after \(timeout)s"
                )
                DispatchQueue.main.async(execute: cancelController)
            }
        }

        guard let result = lock.withLock({ result }) else {
            throw PasskeyError.RequestFailed(
                operation: context.operation,
                reason: .unknown(diagnosticMessage: "no result received from delegate")
            )
        }
        return try result.get()
    }

    func presentationAnchor(for _: ASAuthorizationController) -> ASPresentationAnchor {
        lock.withLock {
            didRequestPresentationAnchor = true
        }

        return anchor
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        if complete(with: .success(authorization.credential)) {
            Log.info("[PASSKEY] \(context.logDescription) completed credential_type=\(type(of: authorization.credential))")
        }
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        let didRequestPresentationAnchor = lock.withLock {
            self.didRequestPresentationAnchor
        }
        let result: Result<ASAuthorizationCredential, Error>
        let logMessage: String
        let shouldWarn: Bool
        let errorMetadata = passkeyNSErrorMetadata(error)
        let errorLogLine = passkeyErrorLogLine(error)

        switch error as? ASAuthorizationError {
        case let authError?:
            switch passkeyAuthorizationFailure(
                for: authError.code,
                didRequestPresentationAnchor: didRequestPresentationAnchor,
                diagnosticMessage: errorMetadata
            ) {
            case .notCompleted:
                result = .failure(PasskeyError.NotCompleted)
                logMessage = "[PASSKEY] \(context.logDescription) cancelled \(errorLogLine)"
                shouldWarn = false
            case let .requestFailed(reason):
                result = .failure(
                    PasskeyError.RequestFailed(
                        operation: context.operation,
                        reason: reason
                    )
                )
                logMessage = "[PASSKEY] \(context.logDescription) failed requested_ui=\(didRequestPresentationAnchor) \(errorLogLine)"
                shouldWarn = true
            }
        case nil:
            result = .failure(
                PasskeyError.RequestFailed(
                    operation: context.operation,
                    reason: .unknown(diagnosticMessage: errorMetadata)
                )
            )
            logMessage = "[PASSKEY] \(context.logDescription) failed with non-auth error \(errorLogLine)"
            shouldWarn = true
        }

        guard complete(with: result) else { return }

        if shouldWarn {
            Log.warn(logMessage)
        } else {
            Log.info(logMessage)
        }
    }

    @discardableResult
    private func complete(
        with result: Result<ASAuthorizationCredential, Error>
    ) -> Bool {
        let didComplete = lock.withLock {
            guard self.result == nil else { return false }

            self.result = result
            return true
        }

        if didComplete {
            semaphore.signal()
        }

        return didComplete
    }
}

enum PasskeyAuthorizationFailure {
    case notCompleted
    case requestFailed(PasskeyFailureReason)
}

func passkeyAuthorizationFailure(
    for code: ASAuthorizationError.Code,
    didRequestPresentationAnchor: Bool,
    diagnosticMessage: String
) -> PasskeyAuthorizationFailure {
    if code == .canceled {
        return .notCompleted
    }

    return .requestFailed(
        passkeyFailureReason(
            for: code,
            didRequestPresentationAnchor: didRequestPresentationAnchor,
            diagnosticMessage: diagnosticMessage
        )
    )
}

func passkeyFailureReason(
    for code: ASAuthorizationError.Code,
    didRequestPresentationAnchor: Bool,
    diagnosticMessage: String
) -> PasskeyFailureReason {
    switch code {
    case .failed where !didRequestPresentationAnchor:
        .platformAuthorizationFailed
    case .failed:
        .platformAuthorizationFailedAfterPresentation
    case .invalidResponse:
        .invalidResponse
    case .notHandled:
        .notHandled
    case .notInteractive:
        .notHandled
    default:
        .unknown(diagnosticMessage: diagnosticMessage)
    }
}

// MARK: - PasskeyExistenceDelegate

/// Lightweight delegate for non-interactive passkey existence checks
///
/// Only cares about whether the credential exists, not the actual assertion.
/// `.notInteractive` means no matching credential and no UI was shown
private class PasskeyExistenceDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    let semaphore = DispatchSemaphore(value: 0)
    var presence: PasskeyCredentialPresence = .indeterminate
    private let anchor: ASPresentationAnchor
    private var didRequestPresentationAnchor = false

    init(anchor: ASPresentationAnchor) {
        self.anchor = anchor
    }

    func presentationAnchor(for _: ASAuthorizationController) -> ASPresentationAnchor {
        didRequestPresentationAnchor = true
        return anchor
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithAuthorization _: ASAuthorization
    ) {
        presence = .present
        Log.info("[PASSKEY] presence check authorization succeeded")
        semaphore.signal()
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        let errorLogLine = passkeyErrorLogLine(error)

        if let authError = error as? ASAuthorizationError {
            if authError.code == .notInteractive {
                presence = .missing
                Log.info(
                    "[PASSKEY] presence check classified missing requested_ui=\(didRequestPresentationAnchor) \(errorLogLine)"
                )
            } else if authError.code == .canceled, !didRequestPresentationAnchor {
                presence = .missing
                Log.info(
                    "[PASSKEY] presence check classified missing after silent cancellation requested_ui=\(didRequestPresentationAnchor) \(errorLogLine)"
                )
            } else {
                Log.warn(
                    "[PASSKEY] presence check failed with auth error requested_ui=\(didRequestPresentationAnchor) \(errorLogLine)"
                )
            }
        } else {
            Log.warn("[PASSKEY] presence check failed with non-auth error \(errorLogLine)")
        }
        semaphore.signal()
    }
}
