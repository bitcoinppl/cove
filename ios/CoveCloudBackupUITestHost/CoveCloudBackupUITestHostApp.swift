import SwiftUI

@main
struct CoveCloudBackupUITestHostApp: App {
    @UIApplicationDelegateAdaptor(CoveAppDelegate.self) private var appDelegate

    private let root: CoveApplicationRoot

    init() {
        Self.resetLocalStateIfRequested()

        root = CoveApplicationRoot(dependencies: Self.dependencies())
    }

    private static func dependencies() -> CoveApplicationDependencies {
        // a test that names a control directory drives cloud storage, passkeys and the network from
        // its script and keeps the keychain in that directory, so the app can be killed and relaunched
        if let controller = CloudBackupUITestController.fromEnvironment() {
            return CoveApplicationDependencies(
                keychain: ScriptedKeychainAccess(storeURL: controller.keychainURL, controller: controller),
                device: DeviceAccesor(),
                connectivity: ControlledConnectivityAccess(controller: controller),
                passkey: ControlledPasskeyProvider(controller: controller),
                cloudStorage: ControlledCloudStorageAccess(controller: controller)
            )
        }

        let scenario = CloudBackupUITestScenario.current()
        let passkey: any PasskeyProvider =
            if scenario == .nativePasskeySmoke {
                PasskeyProviderImpl()
            } else {
                ScriptedPasskeyProvider(scenario: scenario)
            }

        return CoveApplicationDependencies(
            keychain: ScriptedKeychainAccess(),
            device: DeviceAccesor(),
            connectivity: CloudConnectivityMonitor.shared,
            passkey: passkey,
            cloudStorage: ScriptedCloudStorageAccess(scenario: scenario)
        )
    }

    private static func resetLocalStateIfRequested() {
        guard ProcessInfo.processInfo.environment["COVE_CLOUD_BACKUP_UI_RESET"] == "1" else {
            return
        }

        // the diagnostics logs stay: a class's tests share one simulator session, and its
        // failure artifacts hold the app logs of every test in it, which append and rotate
        let root = rootDataDirPath()
        let entries = (try? FileManager.default.contentsOfDirectory(atPath: root)) ?? []
        for entry in entries where entry != "logs" {
            try? FileManager.default.removeItem(atPath: (root as NSString).appendingPathComponent(entry))
        }

        // the scripted keychain is local state too, while the cloud copy in the control
        // directory is not, so a reset wipes only the keychain
        if let directory = ProcessInfo.processInfo.environment[CloudBackupUITestControl.environmentKey] {
            try? FileManager.default.removeItem(
                atPath: (directory as NSString).appendingPathComponent(CloudBackupUITestControl.keychainFileName)
            )
        }
    }

    var body: some Scene {
        WindowGroup {
            root
                .task { await CloudBackupUITestStateProbe.start() }
        }
    }
}
