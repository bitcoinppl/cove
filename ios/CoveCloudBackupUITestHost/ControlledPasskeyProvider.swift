import CoveCore
import Foundation

/// Passkey provider whose results come from the UI test's script and whose calls the test can count
///
/// It returns the same credential ID and PRF output as `ScriptedPasskeyProvider`, which the
/// fixture master key is wrapped with, and the same output on every launch, so a relaunched app
/// can unlock what an earlier process created
final class ControlledPasskeyProvider: PasskeyProvider, @unchecked Sendable {
    private let controller: CloudBackupUITestController
    private static let credentialId = Data(CloudBackupUITestControl.passkeyCredentialId.utf8)
    private let prfOutput = Data((1 ... 32).map(UInt8.init))

    init(controller: CloudBackupUITestController) {
        self.controller = controller
    }

    func createPasskey(
        rpId _: String,
        challenge _: Data,
        user _: PasskeyRegistrationUser
    ) throws -> PasskeyRegistrationResult {
        let index = controller.record(.passkeyCreate)
        try Self.throwIfFailed(controller.script().createResults, index: index, operation: .registration)

        return PasskeyRegistrationResult(
            credentialId: Self.credentialId,
            providerAaguid: "00000000-0000-0000-0000-000000000000",
            registeredPlatform: .ios
        )
    }

    func authenticateWithPrf(
        rpId _: String,
        credentialId requestedCredentialId: Data,
        prfSalt _: Data,
        challenge _: Data
    ) throws -> Data {
        let index = controller.record(.passkeyAuthenticate)
        controller.recordAuthenticatedCredential(requestedCredentialId)
        try Self.throwIfFailed(
            controller.script().authenticateResults,
            index: index,
            operation: .authenticateAssertion
        )

        return prfOutput
    }

    func discoverAndAuthenticateWithPrf(
        rpId _: String,
        prfSalt _: Data,
        challenge _: Data
    ) throws -> DiscoveredPasskeyResult {
        let index = controller.record(.passkeyDiscover)
        try Self.throwIfFailed(controller.script().discoverResults, index: index, operation: .discoverAssertion)

        return DiscoveredPasskeyResult(prfOutput: prfOutput, credentialId: Self.credentialId)
    }

    func isPrfSupported() -> Bool {
        true
    }

    func checkPasskeyPresence(rpId _: String, credentialId _: Data) -> PasskeyCredentialPresence {
        .present
    }

    private static func throwIfFailed(
        _ results: [CloudBackupUITestScript.PasskeyResult],
        index: Int,
        operation: PasskeyOperation
    ) throws {
        guard !results.isEmpty else { return }

        switch results[min(index, results.count - 1)] {
        case .success:
            return
        case .prePresentationFailure:
            throw PasskeyError.RequestFailed(operation: operation, reason: .platformAuthorizationFailed)
        case .postPresentationFailure:
            throw PasskeyError.RequestFailed(
                operation: operation,
                reason: .platformAuthorizationFailedAfterPresentation
            )
        case .userCancelled:
            throw PasskeyError.UserCancelled
        case .noCredential:
            throw PasskeyError.NoCredentialFound
        }
    }
}
