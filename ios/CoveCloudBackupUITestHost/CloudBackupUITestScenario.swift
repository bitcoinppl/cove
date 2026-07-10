import Foundation

enum CloudBackupUITestScenario: String {
    case inventoryUnion = "SC-U02"
    case timeoutThenRetry = "SC-U03"
    case nativePasskeySmoke = "MAN-03"

    static func current(environment: [String: String] = ProcessInfo.processInfo.environment) -> Self {
        guard let rawValue = environment["COVE_CLOUD_BACKUP_UI_SCENARIO"],
              let scenario = Self(rawValue: rawValue)
        else { return .inventoryUnion }

        return scenario
    }
}
