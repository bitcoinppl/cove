import CoveCore
import Foundation

final class ScriptedPasskeyProvider: PasskeyProvider, @unchecked Sendable {
    private let credentialId = Data("cove-cloud-backup-ui-test".utf8)
    private let prfOutput = Data((1 ... 32).map(UInt8.init))

    init(scenario _: CloudBackupUITestScenario) {}

    func createPasskey(
        rpId _: String,
        challenge _: Data,
        user _: PasskeyRegistrationUser
    ) throws -> PasskeyRegistrationResult {
        PasskeyRegistrationResult(
            credentialId: credentialId,
            providerAaguid: "00000000-0000-0000-0000-000000000000",
            registeredPlatform: .ios
        )
    }

    func authenticateWithPrf(
        rpId _: String,
        credentialId _: Data,
        prfSalt _: Data,
        challenge _: Data
    ) throws -> Data {
        prfOutput
    }

    func discoverAndAuthenticateWithPrf(
        rpId _: String,
        prfSalt _: Data,
        challenge _: Data
    ) throws -> DiscoveredPasskeyResult {
        DiscoveredPasskeyResult(prfOutput: prfOutput, credentialId: credentialId)
    }

    func isPrfSupported() -> Bool {
        true
    }

    func checkPasskeyPresence(rpId _: String, credentialId _: Data) -> PasskeyCredentialPresence {
        .present
    }
}
