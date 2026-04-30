import AuthenticationServices

@_exported import CoveCore
import Foundation

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

    private func credentialSummary(_ credentialId: Data) -> String {
        let prefix = credentialId.prefix(4).map { String(format: "%02x", $0) }.joined()
        return "len=\(credentialId.count) prefix=\(prefix)"
    }

    /// PRF is guaranteed on iOS 18.4+ (our minimum deployment target)
    func isPrfSupported() -> Bool {
        true
    }

    func createPasskey(rpId: String, userId: Data, challenge: Data) throws -> Data {
        precondition(!Thread.isMainThread, "createPasskey must not be called from the main thread")

        let registration = try performRegistrationRequest(
            rpId: rpId,
            userId: userId,
            challenge: challenge
        )
        _ = try validateRegistrationPrfMetadata(registration)
        return registration.credentialID
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
            context: "authenticate"
        )
        return prfOutput
    }

    func checkPasskeyPresence(rpId: String, credentialId: Data) -> PasskeyCredentialPresence {
        precondition(
            !Thread.isMainThread,
            "checkPasskeyPresence must not be called from the main thread"
        )

        let credentialSummary = credentialSummary(credentialId)
        Log.info("[PASSKEY] presence check start rpId=\(rpId) credential=\(credentialSummary)")

        let delegate = PasskeyExistenceDelegate()
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

        // .notInteractive returns almost instantly when no credential exists.
        // if iOS doesn't respond quickly enough to prove presence or absence,
        // treat the result as indeterminate instead of assuming success.
        let gotResult = delegate.semaphore.wait(timeout: .now() + 1.0)

        if gotResult == .timedOut {
            Log.warn(
                "[PASSKEY] presence check timed out after 1s rpId=\(rpId) credential=\(credentialSummary)"
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
            context: "discover"
        )
        return DiscoveredPasskeyResult(
            prfOutput: prfOutput,
            credentialId: assertion.credentialID
        )
    }

    private func performRegistrationRequest(
        rpId: String,
        userId: Data,
        challenge: Data
    ) throws -> ASAuthorizationPlatformPublicKeyCredentialRegistration {
        let delegate = PasskeyDelegate(context: "registration")
        let controller: ASAuthorizationController

        controller = DispatchQueue.main.sync {
            Log.info("[PASSKEY] registration request start rpId=\(rpId)")
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: rpId
            )

            let request = provider.createCredentialRegistrationRequest(
                challenge: challenge,
                name: "Cove Wallet",
                userID: userId
            )
            request.prf = .checkForSupport

            let ctrl = ASAuthorizationController(authorizationRequests: [request])
            ctrl.delegate = delegate
            ctrl.presentationContextProvider = delegate
            ctrl.performRequests()
            return ctrl
        }

        _ = controller
        let credential = try delegate.waitForResult()

        guard
            let registration =
            credential as? ASAuthorizationPlatformPublicKeyCredentialRegistration
        else {
            throw PasskeyError.CreationFailed("unexpected credential type")
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
        context: String
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
                "[PASSKEY] \(context) could not obtain usable PRF output: \(error.logDescription)"
            )
            throw PasskeyError.PrfUnsupportedProvider
        }
    }

    private func performAssertionRequest(
        rpId: String,
        credentialId: Data?,
        prfSalt: Data,
        challenge: Data,
        context: String
    ) throws -> ASAuthorizationPlatformPublicKeyCredentialAssertion {
        let delegate = PasskeyDelegate(context: "\(context) assertion")
        let controller: ASAuthorizationController

        controller = DispatchQueue.main.sync {
            Log.info(
                "[PASSKEY] \(context) assertion request start rpId=\(rpId) targeted=\(credentialId != nil)"
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

            request.prf = .inputValues(.init(saltInput1: prfSalt))

            let ctrl = ASAuthorizationController(authorizationRequests: [request])
            ctrl.delegate = delegate
            ctrl.presentationContextProvider = delegate
            ctrl.performRequests()
            return ctrl
        }

        _ = controller
        let credential = try delegate.waitForResult()

        guard
            let assertion =
            credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion
        else {
            if credentialId == nil {
                throw PasskeyError.NoCredentialFound
            }
            throw PasskeyError.AuthenticationFailed("unexpected credential type")
        }

        Log.info("[PASSKEY] \(context) assertion request succeeded credential_len=\(assertion.credentialID.count)")
        return assertion
    }

    private func extractPrfOutput(
        from assertion: ASAuthorizationPlatformPublicKeyCredentialAssertion,
        context: String
    ) throws -> Data {
        if assertion.prf == nil {
            Log.error("[PASSKEY] \(context) assertion PRF output is missing")
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

private func passkeyPresentationAnchor() -> ASPresentationAnchor {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    let activeScene = scenes.first { $0.activationState == .foregroundActive }
    let foregroundScene = activeScene ?? scenes.first { $0.activationState == .foregroundInactive }

    if let window = foregroundScene?.windows.first(where: \.isKeyWindow) {
        return window
    }

    if let window = foregroundScene?.windows.first(where: {
        !$0.isHidden && $0.windowLevel == .normal
    }) {
        return window
    }

    for scene in scenes {
        if let window = scene.windows.first(where: \.isKeyWindow) {
            return window
        }

        if let window = scene.windows.first(where: {
            !$0.isHidden && $0.windowLevel == .normal
        }) {
            return window
        }
    }

    Log.warn("[PASSKEY] no foreground presentation anchor found")
    return ASPresentationAnchor()
}

private class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    private let semaphore = DispatchSemaphore(value: 0)
    private var result: Result<ASAuthorizationCredential, Error>?
    private let context: String

    init(context: String) {
        self.context = context
    }

    func waitForResult() throws -> ASAuthorizationCredential {
        let status = semaphore.wait(timeout: .now() + 120)
        if status == .timedOut {
            Log.error("[PASSKEY] \(context) timed out after 120s")
            throw PasskeyError.AuthenticationFailed("passkey operation timed out after 120s")
        }
        guard let result else { throw PasskeyError.AuthenticationFailed("no result received from delegate") }
        return try result.get()
    }

    func presentationAnchor(for _: ASAuthorizationController) -> ASPresentationAnchor {
        passkeyPresentationAnchor()
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        Log.info("[PASSKEY] \(context) completed credential_type=\(type(of: authorization.credential))")
        result = .success(authorization.credential)
        semaphore.signal()
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        switch error as? ASAuthorizationError {
        case let authError?:
            switch authError.code {
            case .canceled:
                Log.info(
                    "[PASSKEY] \(context) cancelled code=\(authError.code.rawValue) description=\(error.localizedDescription)"
                )
                result = .failure(PasskeyError.UserCancelled)
            default:
                Log.warn(
                    "[PASSKEY] \(context) failed code=\(authError.code.rawValue) description=\(error.localizedDescription)"
                )
                result = .failure(
                    PasskeyError.AuthenticationFailed(error.localizedDescription)
                )
            }
        case nil:
            Log.warn("[PASSKEY] \(context) failed with non-auth error: \(error.localizedDescription)")
            result = .failure(
                PasskeyError.AuthenticationFailed(error.localizedDescription)
            )
        }
        semaphore.signal()
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
    private var didRequestPresentationAnchor = false

    func presentationAnchor(for _: ASAuthorizationController) -> ASPresentationAnchor {
        didRequestPresentationAnchor = true
        return passkeyPresentationAnchor()
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
        if let authError = error as? ASAuthorizationError {
            if authError.code == .notInteractive {
                presence = .missing
                Log.info(
                    "[PASSKEY] presence check classified missing code=\(authError.code.rawValue) requested_ui=\(didRequestPresentationAnchor) description=\(error.localizedDescription)"
                )
            } else if authError.code == .canceled, !didRequestPresentationAnchor {
                presence = .missing
                Log.info(
                    "[PASSKEY] presence check classified missing after silent cancellation code=\(authError.code.rawValue) requested_ui=\(didRequestPresentationAnchor) description=\(error.localizedDescription)"
                )
            } else {
                Log.warn(
                    "[PASSKEY] presence check failed with auth error code=\(authError.code.rawValue) requested_ui=\(didRequestPresentationAnchor) description=\(error.localizedDescription)"
                )
            }
        } else {
            Log.warn("[PASSKEY] presence check failed with non-auth error: \(error.localizedDescription)")
        }
        semaphore.signal()
    }
}
