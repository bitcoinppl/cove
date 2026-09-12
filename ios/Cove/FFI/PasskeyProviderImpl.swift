import AuthenticationServices

@_exported import CoveCore
import Foundation

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

    var requestMode: PasskeyRequestMode {
        switch self {
        case .registration:
            .registration
        case .discoverAssertion:
            .discovery
        case .authenticateAssertion:
            .targeted
        }
    }
}

enum PasskeyRequestMode: String, Equatable {
    case registration
    case discovery
    case targeted
    case presence
}

final class PasskeyRequestDiagnostics: @unchecked Sendable {
    let requestID = UUID()
    let rpId: String
    let operation: String
    let requestMode: PasskeyRequestMode

    private let lock = NSLock()
    private var nativeSubmissionTime: ContinuousClock.Instant?
    private var presentationAnchorTime: ContinuousClock.Instant?
    private var presentationAnchorAvailable: Bool?
    private var presentationSceneActivation: String?
    private var completionTime: ContinuousClock.Instant?

    init(rpId: String, operation: String, requestMode: PasskeyRequestMode) {
        self.rpId = rpId
        self.operation = operation
        self.requestMode = requestMode
    }

    var presentationAnchorRequested: Bool {
        lock.withLock { presentationAnchorTime != nil }
    }

    func markNativeSubmission(at time: ContinuousClock.Instant = ContinuousClock.now) {
        lock.withLock {
            nativeSubmissionTime = time
        }
    }

    func markPresentationAnchorRequest(
        at time: ContinuousClock.Instant = ContinuousClock.now,
        isAvailable: Bool? = nil,
        sceneActivation: String? = nil
    ) {
        lock.withLock {
            if presentationAnchorTime == nil {
                presentationAnchorTime = time
                presentationAnchorAvailable = isAvailable
                presentationSceneActivation = sceneActivation
            }
        }
    }

    func markCompletion(at time: ContinuousClock.Instant = ContinuousClock.now) {
        lock.withLock {
            if completionTime == nil {
                completionTime = time
            }
        }
    }

    func logFields() -> String {
        lock.withLock {
            let submissionToAnchor = durationMilliseconds(
                from: nativeSubmissionTime,
                to: presentationAnchorTime
            )
            let submissionToCompletion = durationMilliseconds(
                from: nativeSubmissionTime,
                to: completionTime
            )
            let anchorAvailable = presentationAnchorAvailable.map { String($0) } ?? "na"
            let sceneActivation = presentationSceneActivation ?? "na"

            return "request_id=\(requestID.uuidString) " +
                "rpId=\(rpId) " +
                "operation=\(operation) " +
                "request_mode=\(requestMode.rawValue) " +
                "submission_to_anchor_ms=\(submissionToAnchor) " +
                "submission_to_completion_ms=\(submissionToCompletion) " +
                "presentation_anchor_requested=\(presentationAnchorTime != nil) " +
                "presentation_anchor_available=\(anchorAvailable) " +
                "presentation_scene_activation=\(sceneActivation)"
        }
    }
}

private func durationMilliseconds(
    from start: ContinuousClock.Instant?,
    to end: ContinuousClock.Instant?
) -> String {
    guard let start, let end else { return "na" }

    let components = start.duration(to: end).components
    let milliseconds = components.seconds * 1000 +
        components.attoseconds / 1_000_000_000_000_000
    return String(milliseconds)
}

func passkeyNSErrorMetadata(_ error: Error) -> String {
    let nsError = error as NSError
    return "error_domain=\(nsError.domain) error_code=\(nsError.code)"
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

    /// PRF is guaranteed on iOS 18.4+ (our minimum deployment target)
    func isPrfSupported() -> Bool {
        true
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

        // passkey authorization requests can present iOS UI, so do not use this for background polling
        let delegate = PasskeyExistenceDelegate(rpId: rpId)
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
            delegate.diagnostics.markNativeSubmission()
            Log.info(
                "[PASSKEY] native request submitted \(delegate.diagnostics.logFields())"
            )
            ctrl.performRequests(options: .preferImmediatelyAvailableCredentials)
            return ctrl
        }

        // .notInteractive returns almost instantly when no credential exists.
        // if iOS doesn't respond quickly enough to prove presence or absence,
        // treat the result as indeterminate instead of assuming success.
        let gotResult = delegate.semaphore.wait(timeout: .now() + 1.0)

        if gotResult == .timedOut {
            delegate.diagnostics.markCompletion()
            Log.warn(
                "[PASSKEY] \(delegate.diagnostics.logFields()) timed_out_after_s=1"
            )
            DispatchQueue.main.async { controller.cancel() }
            return .indeterminate
        }

        Log.info(
            "[PASSKEY] \(delegate.diagnostics.logFields()) presence=\(delegate.presence)"
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
        let delegate = PasskeyDelegate(context: .registration, rpId: rpId)
        let controller: ASAuthorizationController

        controller = DispatchQueue.main.sync {
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
            delegate.diagnostics.markNativeSubmission()
            Log.info(
                "[PASSKEY] native request submitted \(delegate.diagnostics.logFields())"
            )
            ctrl.performRequests()
            return ctrl
        }

        let credential = try delegate.waitForResult {
            controller.cancel()
        }

        guard
            let registration =
            credential as? ASAuthorizationPlatformPublicKeyCredentialRegistration
        else {
            throw passkeyUnexpectedCredentialError(operation: .registration)
        }

        Log.info(
            "[PASSKEY] registration request succeeded \(delegate.diagnostics.logFields())"
        )
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
        let delegate = PasskeyDelegate(context: context, rpId: rpId)
        let controller: ASAuthorizationController

        controller = DispatchQueue.main.sync {
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
            delegate.diagnostics.markNativeSubmission()
            Log.info(
                "[PASSKEY] native request submitted \(delegate.diagnostics.logFields())"
            )
            ctrl.performRequests()
            return ctrl
        }

        let credential = try delegate.waitForResult {
            controller.cancel()
        }

        guard
            let assertion =
            credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion
        else {
            throw passkeyUnexpectedCredentialError(operation: context.operation)
        }

        Log.info(
            "[PASSKEY] \(context.logDescription) request succeeded \(delegate.diagnostics.logFields())"
        )
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

private struct PasskeyPresentationAnchorResolution {
    let anchor: ASPresentationAnchor
    let isAvailable: Bool
    let sceneActivation: String
}

private func passkeySceneActivationName(_ state: UIScene.ActivationState?) -> String {
    guard let state else { return "none" }

    switch state {
    case .foregroundActive:
        return "foregroundActive"
    case .foregroundInactive:
        return "foregroundInactive"
    case .background:
        return "background"
    case .unattached:
        return "unattached"
    @unknown default:
        return "unknown"
    }
}

private func passkeyPresentationAnchor() -> PasskeyPresentationAnchorResolution {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    let activeScene = scenes.first { $0.activationState == .foregroundActive }
    let foregroundScene = activeScene ?? scenes.first { $0.activationState == .foregroundInactive }

    func resolution(for window: UIWindow?, scene: UIWindowScene?) -> PasskeyPresentationAnchorResolution {
        guard let window else {
            return PasskeyPresentationAnchorResolution(
                anchor: ASPresentationAnchor(),
                isAvailable: false,
                sceneActivation: passkeySceneActivationName(
                    scene?.activationState ?? foregroundScene?.activationState
                )
            )
        }

        return PasskeyPresentationAnchorResolution(
            anchor: window,
            isAvailable: true,
            sceneActivation: passkeySceneActivationName(
                window.windowScene?.activationState ?? scene?.activationState
            )
        )
    }

    if let window = foregroundScene?.windows.first(where: \.isKeyWindow) {
        return resolution(for: window, scene: foregroundScene)
    }

    if let window = foregroundScene?.windows.first(where: {
        !$0.isHidden && $0.windowLevel == .normal
    }) {
        return resolution(for: window, scene: foregroundScene)
    }

    for scene in scenes {
        if let window = scene.windows.first(where: \.isKeyWindow) {
            return resolution(for: window, scene: scene)
        }

        if let window = scene.windows.first(where: {
            !$0.isHidden && $0.windowLevel == .normal
        }) {
            return resolution(for: window, scene: scene)
        }
    }

    return resolution(for: nil, scene: foregroundScene)
}

final class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    static let interactiveRequestTimeout: TimeInterval = 5 * 60

    private let semaphore = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private let timeout: TimeInterval
    private var result: Result<ASAuthorizationCredential, Error>?
    private let context: PasskeyOperationContext
    let diagnostics: PasskeyRequestDiagnostics

    init(
        context: PasskeyOperationContext,
        rpId: String = "unknown",
        timeout: TimeInterval = PasskeyDelegate.interactiveRequestTimeout
    ) {
        self.context = context
        self.timeout = timeout
        diagnostics = PasskeyRequestDiagnostics(
            rpId: rpId,
            operation: context.requestMode.rawValue,
            requestMode: context.requestMode
        )
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
                diagnostics.markCompletion()
                Log.warn(
                    "[PASSKEY] \(diagnostics.logFields()) timed_out_after_s=\(timeout)"
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
        let resolution = passkeyPresentationAnchor()
        diagnostics.markPresentationAnchorRequest(
            isAvailable: resolution.isAvailable,
            sceneActivation: resolution.sceneActivation
        )
        Log.info("[PASSKEY] \(diagnostics.logFields())")

        return resolution.anchor
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        diagnostics.markCompletion()
        guard complete(with: .success(authorization.credential)) else {
            Log.info("[PASSKEY] \(diagnostics.logFields()) late_callback")
            return
        }

        Log.info(
            "[PASSKEY] \(diagnostics.logFields()) completed"
        )
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        diagnostics.markCompletion()
        let presentationAnchorRequested = diagnostics.presentationAnchorRequested
        let result: Result<ASAuthorizationCredential, Error>
        let logMessage: String
        let shouldWarn: Bool
        let errorMetadata = passkeyNSErrorMetadata(error)

        switch error as? ASAuthorizationError {
        case let authError?:
            switch passkeyAuthorizationFailure(
                for: authError.code,
                didRequestPresentationAnchor: presentationAnchorRequested,
                diagnosticMessage: errorMetadata
            ) {
            case .userCancelled:
                result = .failure(PasskeyError.UserCancelled)
                logMessage = "[PASSKEY] \(diagnostics.logFields()) cancelled \(errorMetadata)"
                shouldWarn = false
            case let .requestFailed(reason):
                result = .failure(
                    PasskeyError.RequestFailed(
                        operation: context.operation,
                        reason: reason
                    )
                )
                logMessage = "[PASSKEY] \(diagnostics.logFields()) failed \(errorMetadata)"
                shouldWarn = true
            }
        case nil:
            result = .failure(
                PasskeyError.RequestFailed(
                    operation: context.operation,
                    reason: .unknown(diagnosticMessage: errorMetadata)
                )
            )
            logMessage = "[PASSKEY] \(diagnostics.logFields()) failed_non_auth \(errorMetadata)"
            shouldWarn = true
        }

        guard complete(with: result) else {
            Log.info("[PASSKEY] \(diagnostics.logFields()) late_callback \(errorMetadata)")
            return
        }

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
    case userCancelled
    case requestFailed(PasskeyFailureReason)
}

func passkeyAuthorizationFailure(
    for code: ASAuthorizationError.Code,
    didRequestPresentationAnchor: Bool,
    diagnosticMessage: String
) -> PasskeyAuthorizationFailure {
    if code == .canceled {
        return .userCancelled
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
    let diagnostics: PasskeyRequestDiagnostics

    init(rpId: String) {
        diagnostics = PasskeyRequestDiagnostics(
            rpId: rpId,
            operation: PasskeyRequestMode.presence.rawValue,
            requestMode: .presence
        )
    }

    func presentationAnchor(for _: ASAuthorizationController) -> ASPresentationAnchor {
        let resolution = passkeyPresentationAnchor()
        diagnostics.markPresentationAnchorRequest(
            isAvailable: resolution.isAvailable,
            sceneActivation: resolution.sceneActivation
        )
        Log.info("[PASSKEY] \(diagnostics.logFields())")
        return resolution.anchor
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithAuthorization _: ASAuthorization
    ) {
        diagnostics.markCompletion()
        presence = .present
        Log.info("[PASSKEY] \(diagnostics.logFields()) presence=\(presence)")
        semaphore.signal()
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        diagnostics.markCompletion()
        let presentationAnchorRequested = diagnostics.presentationAnchorRequested
        let errorMetadata = passkeyNSErrorMetadata(error)

        if let authError = error as? ASAuthorizationError {
            if authError.code == .notInteractive {
                presence = .missing
                Log.info(
                    "[PASSKEY] \(diagnostics.logFields()) classified=missing \(errorMetadata)"
                )
            } else if authError.code == .canceled, !presentationAnchorRequested {
                presence = .missing
                Log.info(
                    "[PASSKEY] \(diagnostics.logFields()) classified=missing_after_silent_cancellation \(errorMetadata)"
                )
            } else {
                Log.warn(
                    "[PASSKEY] \(diagnostics.logFields()) failed \(errorMetadata)"
                )
            }
        } else {
            Log.warn(
                "[PASSKEY] \(diagnostics.logFields()) failed_non_auth \(errorMetadata)"
            )
        }
        semaphore.signal()
    }
}
