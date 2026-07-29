import SwiftUI

enum AboutAlertState: Equatable {
    case confirmBetaEnable
    case confirmBetaDisable
    case betaEnabled
    case betaError(String)
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
    @State private var isSubmittedDiagnosticsPresented = false
    @State private var submittedDiagnosticsLoadState: SubmittedDiagnosticsLoadState = .loaded([])

    private var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
    }

    private var buildNumber: String {
        Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? ""
    }

    private var gitBranch: String {
        #if DEBUG
            app.rust.gitBranch()
        #else
            ""
        #endif
    }

    private var presentationContext: AboutPresentationContext {
        AboutPresentationContext(
            alertState: $alertState,
            isBetaEnabled: $isBetaEnabled,
            dismiss: { dismiss() }
        )
    }

    var body: some View {
        Form {
            AboutVersionSection(
                appVersion: appVersion,
                buildNumber: buildNumber,
                gitCommit: app.rust.gitShortHash(),
                gitBranch: gitBranch,
                onBuildNumberTapped: handleBuildNumberTap
            )

            AboutSupportSection(
                isInDecoyMode: auth.isInDecoyMode(),
                submittedDiagnosticsLoadState: submittedDiagnosticsLoadState,
                onSendDiagnostics: presentSendDiagnostics,
                onSubmittedDiagnostics: presentSubmittedDiagnostics
            )
        }
        .navigationTitle("About")
        .task { refreshSubmittedDiagnostics() }
        .onChange(of: isSendDiagnosticsPresented) { _, isPresented in
            guard !isPresented else { return }

            refreshSubmittedDiagnostics()
        }
        .onChange(of: auth.isInDecoyMode()) { _, _ in
            refreshSubmittedDiagnostics()
        }
        .onDisappear { buildTapTimer?.invalidate(); buildTapTimer = nil }
        .sheet(isPresented: $isSendDiagnosticsPresented) {
            SendDiagnosticsSheet()
        }
        .sheet(isPresented: $isSubmittedDiagnosticsPresented, onDismiss: refreshSubmittedDiagnostics) {
            SubmittedDiagnosticsSheet(
                loadState: submittedDiagnosticsLoadState,
                onRecordsChanged: refreshSubmittedDiagnostics
            )
        }
        .presentingAlert($alertState, context: presentationContext, defaultTitle: "Error")
    }

    private func handleBuildNumberTap() {
        buildTapCount += 1
        buildTapTimer?.invalidate()
        buildTapTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: false) { _ in
            buildTapCount = 0
        }

        guard buildTapCount >= 5 else { return }

        buildTapCount = 0
        buildTapTimer?.invalidate()
        alertState = .init(isBetaEnabled ? .confirmBetaDisable : .confirmBetaEnable)
    }

    private func presentSendDiagnostics() {
        isSendDiagnosticsPresented = true
    }

    private func presentSubmittedDiagnostics() {
        isSubmittedDiagnosticsPresented = true
    }

    private func refreshSubmittedDiagnostics() {
        guard !auth.isInDecoyMode() else {
            submittedDiagnosticsLoadState = .loaded([])
            isSubmittedDiagnosticsPresented = false
            return
        }

        Task {
            let loadState = await loadSubmittedDiagnosticsHistory()
            guard !auth.isInDecoyMode() else {
                submittedDiagnosticsLoadState = .loaded([])
                isSubmittedDiagnosticsPresented = false
                return
            }

            submittedDiagnosticsLoadState = loadState
        }
    }
}

private struct AboutVersionSection: View {
    let appVersion: String
    let buildNumber: String
    let gitCommit: String
    let gitBranch: String
    let onBuildNumberTapped: () -> Void

    var body: some View {
        Section {
            AboutInfoRow(title: "Version", value: appVersion)

            AboutInfoRow(title: "Build Number", value: buildNumber)
                .contentShape(Rectangle())
                .onTapGesture(perform: onBuildNumberTapped)

            AboutInfoRow(title: "Git Commit", value: gitCommit)

            #if DEBUG
                AboutInfoRow(title: "Git Branch", value: gitBranch)
            #endif
        }
    }
}

private struct AboutInfoRow: View {
    let title: String
    let value: String

    var body: some View {
        HStack {
            Text(title)
            Spacer()
            Text(value)
                .foregroundStyle(.secondary)
        }
    }
}

private struct AboutSupportSection: View {
    let isInDecoyMode: Bool
    let submittedDiagnosticsLoadState: SubmittedDiagnosticsLoadState
    let onSendDiagnostics: () -> Void
    let onSubmittedDiagnostics: () -> Void

    var body: some View {
        Section {
            FeedbackLink()

            if !isInDecoyMode {
                AboutActionButton(title: "Send Diagnostics", action: onSendDiagnostics)

                if submittedDiagnosticsLoadState.shouldShowSubmittedDiagnostics {
                    SubmittedDiagnosticsButton(
                        summary: submittedDiagnosticsLoadState.submittedDiagnosticsSummary,
                        action: onSubmittedDiagnostics
                    )
                }
            }
        }
    }
}

private struct FeedbackLink: View {
    var body: some View {
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

private struct AboutActionButton: View {
    let title: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack {
                Text(title)
                    .foregroundStyle(.primary)
            }
        }
    }
}

private struct SubmittedDiagnosticsButton: View {
    let summary: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack {
                Text("Submitted Diagnostics")
                    .foregroundStyle(.primary)
                Spacer()
                Text(summary)
                    .foregroundStyle(.secondary)
                    .font(.footnote)
            }
        }
    }
}

#Preview {
    NavigationStack {
        AboutScreen()
            .environment(AppManager.shared)
            .environment(AuthManager.shared)
    }
}
