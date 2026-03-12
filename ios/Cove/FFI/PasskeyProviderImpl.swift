import AuthenticationServices

@_exported import CoveCore
import Foundation

final class PasskeyProviderImpl: PasskeyProvider, @unchecked Sendable {
    func isPrfSupported() -> Bool {
        // PRF is guaranteed on iOS 18.4+ (our minimum deployment target)
        true
    }

    func createPasskey(rpId: String, userId: [UInt8], challenge: [UInt8]) throws -> [UInt8] {
        precondition(!Thread.isMainThread, "createPasskey must not be called from the main thread")

        let delegate = PasskeyDelegate()
        let controller: ASAuthorizationController

        // setup + performRequests must happen on main (UI requirement)
        controller = DispatchQueue.main.sync {
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: rpId
            )

            let request = provider.createCredentialRegistrationRequest(
                challenge: Data(challenge),
                name: "Cove Wallet",
                userID: Data(userId)
            )

            let ctrl = ASAuthorizationController(authorizationRequests: [request])
            ctrl.delegate = delegate
            ctrl.presentationContextProvider = delegate
            ctrl.performRequests()
            return ctrl
        }

        // wait on calling thread (Rust worker) — main is free for delegate callbacks
        _ = controller
        let credential = try delegate.waitForResult()

        guard
            let registration =
            credential as? ASAuthorizationPlatformPublicKeyCredentialRegistration
        else {
            throw PasskeyError.creationFailed("unexpected credential type")
        }

        return Array(registration.credentialID)
    }

    func authenticateWithPrf(
        rpId: String, credentialId: [UInt8], prfSalt: [UInt8], challenge: [UInt8]
    ) throws -> [UInt8] {
        precondition(
            !Thread.isMainThread,
            "authenticateWithPrf must not be called from the main thread"
        )

        let delegate = PasskeyDelegate()
        let controller: ASAuthorizationController

        controller = DispatchQueue.main.sync {
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: rpId
            )

            let request = provider.createCredentialAssertionRequest(
                challenge: Data(challenge)
            )

            request.allowedCredentials = [
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(
                    credentialID: Data(credentialId)
                ),
            ]

            let prfInput = ASAuthorizationPublicKeyCredentialPRFAssertionInput(
                inputValues: ASAuthorizationPublicKeyCredentialPRFValues(
                    saltInput: Data(prfSalt)
                )
            )
            request.prf = prfInput

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
            throw PasskeyError.authenticationFailed("unexpected credential type")
        }

        guard let prfOutput = assertion.prf?.first?.outputValue else {
            throw PasskeyError.authenticationFailed("PRF output not available")
        }

        guard prfOutput.count >= 32 else {
            throw PasskeyError.authenticationFailed(
                "PRF output too short: \(prfOutput.count) bytes, need 32"
            )
        }

        return Array(prfOutput.prefix(32))
    }

    func discoverAndAuthenticateWithPrf(
        rpId: String, prfSalt: [UInt8], challenge: [UInt8]
    ) throws -> DiscoveredPasskeyResult {
        precondition(
            !Thread.isMainThread,
            "discoverAndAuthenticateWithPrf must not be called from the main thread"
        )

        let delegate = PasskeyDelegate()
        let controller: ASAuthorizationController

        controller = DispatchQueue.main.sync {
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: rpId
            )

            let request = provider.createCredentialAssertionRequest(
                challenge: Data(challenge)
            )

            // no allowedCredentials — discoverable credential
            request.allowedCredentials = []

            let prfInput = ASAuthorizationPublicKeyCredentialPRFAssertionInput(
                inputValues: ASAuthorizationPublicKeyCredentialPRFValues(
                    saltInput: Data(prfSalt)
                )
            )
            request.prf = prfInput

            let ctrl = ASAuthorizationController(authorizationRequests: [request])
            ctrl.delegate = delegate
            ctrl.presentationContextProvider = delegate
            ctrl.performRequests(
                options: .preferImmediatelyAvailableCredentials
            )
            return ctrl
        }

        _ = controller
        let credential = try delegate.waitForResult()

        guard
            let assertion =
            credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion
        else {
            throw PasskeyError.noCredentialFound
        }

        guard let prfOutput = assertion.prf?.first?.outputValue else {
            throw PasskeyError.authenticationFailed("PRF output not available")
        }

        guard prfOutput.count >= 32 else {
            throw PasskeyError.authenticationFailed(
                "PRF output too short: \(prfOutput.count) bytes, need 32"
            )
        }

        return DiscoveredPasskeyResult(
            prfOutput: Array(prfOutput.prefix(32)),
            credentialId: Array(assertion.credentialID)
        )
    }
}

// MARK: - PasskeyDelegate

private class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    private let semaphore = DispatchSemaphore(value: 0)
    private var result: Result<ASAuthorizationCredential, Error>?

    func waitForResult() throws -> ASAuthorizationCredential {
        semaphore.wait()
        guard let result else {
            throw PasskeyError.authenticationFailed("no result received from delegate")
        }
        return try result.get()
    }

    func presentationAnchor(for _: ASAuthorizationController) -> ASPresentationAnchor {
        let scenes = UIApplication.shared.connectedScenes
        let windowScene = scenes.first as? UIWindowScene
        return windowScene?.keyWindow ?? ASPresentationAnchor()
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        result = .success(authorization.credential)
        semaphore.signal()
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        if let authError = error as? ASAuthorizationError {
            switch authError.code {
            case .canceled:
                result = .failure(PasskeyError.userCancelled)
            default:
                result = .failure(
                    PasskeyError.authenticationFailed(error.localizedDescription)
                )
            }
        } else {
            result = .failure(
                PasskeyError.authenticationFailed(error.localizedDescription)
            )
        }
        semaphore.signal()
    }
}
