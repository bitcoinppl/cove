import SwiftUI

@main
struct CoveCloudBackupUITestHostApp: App {
    @UIApplicationDelegateAdaptor(CoveAppDelegate.self) private var appDelegate

    private let root: CoveApplicationRoot

    init() {
        let scenario = CloudBackupUITestScenario.current()
        let keychain = ScriptedKeychainAccess()
        let passkey: any PasskeyProvider =
            if scenario == .nativePasskeySmoke {
                PasskeyProviderImpl()
            } else {
                ScriptedPasskeyProvider(scenario: scenario)
            }
        Self.resetLocalStateIfRequested()

        root = CoveApplicationRoot(
            dependencies: CoveApplicationDependencies(
                keychain: keychain,
                device: DeviceAccesor(),
                connectivity: CloudConnectivityMonitor.shared,
                passkey: passkey,
                cloudStorage: ScriptedCloudStorageAccess(scenario: scenario)
            )
        )
    }

    private static func resetLocalStateIfRequested() {
        guard ProcessInfo.processInfo.environment["COVE_CLOUD_BACKUP_UI_RESET"] == "1" else {
            return
        }

        try? FileManager.default.removeItem(atPath: rootDataDirPath())
    }

    var body: some Scene {
        WindowGroup {
            root
        }
    }
}
