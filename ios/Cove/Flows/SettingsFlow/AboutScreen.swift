import SwiftUI

private enum AlertState: Equatable {
    case confirmBetaEnable
    case confirmBetaDisable
    case betaError(String)
}

struct AboutScreen: View {
    @Environment(AppManager.self) private var app

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
                        alertState = .none
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

        case let .betaError(error):
            AlertBuilder(
                title: "Something went wrong!",
                message: error,
                actions: { Button("OK") { alertState = .none } }
            ).eraseToAny()
        }
    }
}

#Preview {
    NavigationStack {
        AboutScreen()
            .environment(AppManager.shared)
    }
}
