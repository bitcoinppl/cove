import SwiftUI

enum AboutAlertState: Equatable {
    case confirmBetaEnable
    case confirmBetaDisable
    case betaEnabled
    case betaError(String)
    case confirmWipeCloud
    case wipeCloudResult(WipeCloudResult)
    case confirmResetLocalState
    case resetLocalStateResult(String)
}

struct WipeCloudResult: Equatable {
    let succeeded: Bool
    let message: String
}

struct AboutScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(\.dismiss) private var dismiss

    @State private var buildTapCount = 0
    @State private var buildTapTimer: Timer? = nil
    @State private var isBetaEnabled = Database().globalFlag().getBoolConfig(key: .betaFeaturesEnabled)
    @State private var alertState: TaggedItem<AboutAlertState>? = nil
    @State private var isSendDiagnosticsPresented = false

    private var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
    }

    private var buildNumber: String {
        Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
    }

    private var presentationContext: AboutPresentationContext {
        AboutPresentationContext(
            alertState: $alertState,
            isBetaEnabled: $isBetaEnabled,
            dismiss: { dismiss() },
            wipeCloudBackup: Self.debugWipeCloudBackup
        )
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

                #if DEBUG
                    HStack {
                        Text("Git Branch")
                        Spacer()
                        Text(app.rust.gitBranch())
                            .foregroundStyle(.secondary)
                    }
                #endif
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

                if !auth.isInDecoyMode() {
                    Button {
                        isSendDiagnosticsPresented = true
                    } label: {
                        HStack {
                            Text("Send Diagnostics")
                                .foregroundStyle(.primary)
                            Spacer()
                            Text("Review before upload")
                                .foregroundStyle(.secondary)
                                .font(.footnote)
                        }
                    }
                }
            }

            #if DEBUG
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
            #endif
        }
        .navigationTitle("About")
        .onDisappear { buildTapTimer?.invalidate(); buildTapTimer = nil }
        .sheet(isPresented: $isSendDiagnosticsPresented) {
            SendDiagnosticsSheet()
        }
        .presentingAlert($alertState, context: presentationContext, defaultTitle: "Error")
    }

    private nonisolated static func debugWipeCloudBackup() -> WipeCloudResult {
        let helper = ICloudDriveHelper.shared

        do {
            let dataDir = try helper.dataDirectoryURL()
            if FileManager.default.fileExists(atPath: dataDir.path) {
                try FileManager.default.removeItem(at: dataDir)
            }
        } catch {
            return WipeCloudResult(
                succeeded: false,
                message: "iCloud wipe failed: \(error.localizedDescription)"
            )
        }

        RustCloudBackupManager().debugResetCloudBackupState()
        return WipeCloudResult(succeeded: true, message: "All cloud backup data deleted and local state reset")
    }
}

#Preview {
    NavigationStack {
        AboutScreen()
            .environment(AppManager.shared)
            .environment(AuthManager.shared)
    }
}
