import AuthenticationServices
import CryptoKit

@_exported import CoveCore
import Foundation

final class PasskeyProviderImpl: PasskeyProvider, @unchecked Sendable {
    func isPrfSupported() -> Bool {
        // PRF is guaranteed on iOS 18.4+ (our minimum deployment target)
        true
    }

    func createPasskey(rpId: String, userId: Data, challenge: Data) throws -> Data {
        precondition(!Thread.isMainThread, "createPasskey must not be called from the main thread")

        let delegate = PasskeyDelegate()
        let controller: ASAuthorizationController

        // setup + performRequests must happen on main (UI requirement)
        controller = DispatchQueue.main.sync {
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
                relyingPartyIdentifier: rpId
            )

            let request = provider.createCredentialRegistrationRequest(
                challenge: challenge,
                name: "Cove Wallet",
                userID: userId
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
            throw PasskeyError.CreationFailed("unexpected credential type")
        }

        return registration.credentialID
    }

    func authenticateWithPrf(
        rpId: String, credentialId: Data, prfSalt: Data, challenge: Data
    ) throws -> Data {
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
                challenge: challenge
            )

            request.allowedCredentials = [
                ASAuthorizationPlatformPublicKeyCredentialDescriptor(
                    credentialID: credentialId
                ),
            ]

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
            throw PasskeyError.AuthenticationFailed("unexpected credential type")
        }

        guard let prfKey = assertion.prf?.first else {
            throw PasskeyError.AuthenticationFailed("PRF output not available")
        }

        let prfOutput = prfKey.withUnsafeBytes { Data($0) }

        guard prfOutput.count >= 32 else {
            throw PasskeyError.AuthenticationFailed(
                "PRF output too short: \(prfOutput.count) bytes, need 32"
            )
        }

        return prfOutput.prefix(32)
    }

    func discoverAndAuthenticateWithPrf(
        rpId: String, prfSalt: Data, challenge: Data
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
                challenge: challenge
            )

            // no allowedCredentials — discoverable credential
            request.allowedCredentials = []

            request.prf = .inputValues(.init(saltInput1: prfSalt))

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
            throw PasskeyError.NoCredentialFound
        }

        guard let prfKey = assertion.prf?.first else {
            throw PasskeyError.AuthenticationFailed("PRF output not available")
        }

        let prfOutput = prfKey.withUnsafeBytes { Data($0) }

        guard prfOutput.count >= 32 else {
            throw PasskeyError.AuthenticationFailed(
                "PRF output too short: \(prfOutput.count) bytes, need 32"
            )
        }

        return DiscoveredPasskeyResult(
            prfOutput: prfOutput.prefix(32),
            credentialId: assertion.credentialID
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
            throw PasskeyError.AuthenticationFailed("no result received from delegate")
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
                result = .failure(PasskeyError.UserCancelled)
            default:
                result = .failure(
                    PasskeyError.AuthenticationFailed(error.localizedDescription)
                )
            }
        } else {
            result = .failure(
                PasskeyError.AuthenticationFailed(error.localizedDescription)
            )
        }
        semaphore.signal()
    }
}
