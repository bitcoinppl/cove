import SwiftUI

private enum AlertState: Equatable {
    case confirmBetaEnable
    case confirmBetaDisable
    case betaEnabled
    case betaError(String)
    case confirmWipeCloud
    case wipeCloudResult(String)
    case confirmResetLocalState
    case resetLocalStateResult(String)
}

struct AboutScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss

    @State private var buildTapCount = 0
    @State private var buildTapTimer: Timer? = nil
    @State private var isBetaEnabled = Database().globalFlag().getBoolConfig(key: .betaFeaturesEnabled)
    @State private var alertState: TaggedItem<AlertState>? = nil

    private var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
    }

    private var buildNumber: String {
        Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
    }

    var body: some View {
        Form {
            Section {
                HStack {
                    Text("Version")
                    Spacer()
                    Text(appVersion)
                        .foregroundStyle(.secondary)
                }

                HStack {
                    Text("Build Number")
                    Spacer()
                    Text(buildNumber)
                        .foregroundStyle(.secondary)
                }
                .contentShape(Rectangle())
                .onTapGesture {
                    buildTapCount += 1
                    buildTapTimer?.invalidate()
                    buildTapTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: false) { _ in
                        buildTapCount = 0
                    }

                    if buildTapCount >= 5 {
                        buildTapCount = 0
                        buildTapTimer?.invalidate()
                        if isBetaEnabled {
                            alertState = .init(.confirmBetaDisable)
                        } else {
                            alertState = .init(.confirmBetaEnable)
                        }
                    }
                }

                HStack {
                    Text("Git Commit")
                    Spacer()
                    Text(app.rust.gitShortHash())
                        .foregroundStyle(.secondary)
                }
            }

            Section {
                Link(destination: URL(string: "mailto:feedback@covebitcoinwallet.com")!) {
                    HStack {
                        Text("Feedback")
                            .foregroundStyle(.primary)
                        Spacer()
                        Text("feedback@covebitcoinwallet.com")
                            .foregroundStyle(.secondary)
                            .font(.footnote)
                    }
                }
            }

            if isBetaEnabled {
                Section("Debug") {
                    Button(role: .destructive) {
                        alertState = .init(.confirmWipeCloud)
                    } label: {
                        Text("Wipe Cloud Backup")
                    }

                    Button {
                        alertState = .init(.confirmResetLocalState)
                    } label: {
                        Text("Reset Local Backup State")
                    }
                }
            }
        }
        .navigationTitle("About")
        .onDisappear { buildTapTimer?.invalidate(); buildTapTimer = nil }
        .alert(
            alertTitle,
            isPresented: showingAlert,
            presenting: alertState,
            actions: { MyAlert($0).actions },
            message: { MyAlert($0).message }
        )
    }

    // MARK: Alerts

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { alertState != nil },
            set: { if !$0 { alertState = .none } }
        )
    }

    private var alertTitle: String {
        guard let alertState else { return "Error" }
        return MyAlert(alertState).title
    }

    private func MyAlert(_ alert: TaggedItem<AlertState>) -> AnyAlertBuilder {
        switch alert.item {
        case .confirmBetaEnable:
            AlertBuilder(
                title: "Enable Beta Features?",
                message: "This will enable experimental features",
                actions: {
                    Button("Enable") {
                        do {
                            try Database().globalFlag().set(key: .betaFeaturesEnabled, value: true)
                            isBetaEnabled = true
                        } catch {
                            alertState = .init(.betaError("Failed to enable beta features: \(error.localizedDescription)"))
                            return
                        }
                        alertState = .init(.betaEnabled)
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case .confirmBetaDisable:
            AlertBuilder(
                title: "Disable Beta Features?",
                message: "This will hide experimental features",
                actions: {
                    Button("Disable") {
                        do {
                            try Database().globalFlag().set(key: .betaFeaturesEnabled, value: false)
                            isBetaEnabled = false
                        } catch {
                            alertState = .init(.betaError("Failed to disable beta features: \(error.localizedDescription)"))
                            return
                        }
                        alertState = .none
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case .betaEnabled:
            AlertBuilder(
                title: "Beta Features Enabled",
                message: "Beta features have been enabled",
                actions: { Button("OK") { dismiss() } }
            ).eraseToAny()

        case let .betaError(error):
            AlertBuilder(
                title: "Something went wrong!",
                message: error,
                actions: { Button("OK") { alertState = .none } }
            ).eraseToAny()

        case .confirmWipeCloud:
            AlertBuilder(
                title: "Wipe Cloud Backup?",
                message: "Deletes all iCloud backup files and resets local backup state",
                actions: {
                    Button("Wipe", role: .destructive) {
                        Task.detached {
                            let result = Self.debugWipeCloudBackup()
                            await MainActor.run {
                                alertState = .init(.wipeCloudResult(result))
                            }
                        }
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case let .wipeCloudResult(message):
            AlertBuilder(
                title: "Cloud Backup Wiped",
                message: message,
                actions: { Button("OK") { alertState = .none } }
            ).eraseToAny()

        case .confirmResetLocalState:
            AlertBuilder(
                title: "Reset Local Backup State?",
                message: "Clears local keychain and DB backup state but keeps iCloud files intact. Use this to test the recovery flow.",
                actions: {
                    Button("Reset", role: .destructive) {
                        RustCloudBackupManager().debugResetCloudBackupState()
                        alertState = .init(.resetLocalStateResult("Local backup state reset. iCloud files are untouched."))
                    }
                    Button("Cancel", role: .cancel) { alertState = .none }
                }
            ).eraseToAny()

        case let .resetLocalStateResult(message):
            AlertBuilder(
                title: "Local State Reset",
                message: message,
                actions: { Button("OK") { alertState = .none } }
            ).eraseToAny()
        }
    }

    private nonisolated static func debugWipeCloudBackup() -> String {
        let helper = ICloudDriveHelper.shared

        do {
            let dataDir = try helper.dataDirectoryURL()
            if FileManager.default.fileExists(atPath: dataDir.path) {
                try FileManager.default.removeItem(at: dataDir)
            }
        } catch {
            return "iCloud wipe failed: \(error.localizedDescription)"
        }

        RustCloudBackupManager().debugResetCloudBackupState()
        return "All cloud backup data deleted and local state reset"
    }
}

#Preview {
    NavigationStack {
        AboutScreen()
            .environment(AppManager.shared)
    }
}
