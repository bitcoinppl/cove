import SwiftUI

struct CatastrophicErrorView: View {
    let onRestoreFromCloud: () -> Void
    let onWipeOnly: () -> Void

    enum CloudProbeState: Equatable {
        case checking
        case available
        case noBackup
        case offline(String)
        case inconclusive(String)
        case unreadable(String)

        var allowsRestoreAttempt: Bool {
            switch self {
            case .available:
                true
            case .checking, .noBackup, .offline, .inconclusive, .unreadable:
                false
            }
        }

        var allowsRetry: Bool {
            switch self {
            case .offline, .inconclusive, .unreadable:
                true
            case .checking, .available, .noBackup:
                false
            }
        }
    }

    private enum RecoveryConfirmation: Identifiable {
        case restore
        case wipe

        var id: String {
            switch self {
            case .restore:
                "restore"
            case .wipe:
                "wipe"
            }
        }

        var title: String {
            switch self {
            case .restore:
                "Restore from Cloud Backup?"
            case .wipe:
                "Wipe All Local Data?"
            }
        }

        var message: String {
            switch self {
            case .restore:
                "Cove found Cloud Backup data for the selected iCloud account. This will erase the damaged local data on this device, then verify your passkey during restore."
            case .wipe:
                "This will permanently delete all wallet data on this device. Make sure you have your recovery phrases backed up. This cannot be undone."
            }
        }

        var actionTitle: String {
            switch self {
            case .restore:
                "Erase and Restore"
            case .wipe:
                "Wipe Data"
            }
        }
    }

    @State private var cloudProbeState: CloudProbeState = .checking
    @State private var cloudProbeTask: Task<Void, Never>?
    @State private var recoveryConfirmation: RecoveryConfirmation?

    var body: some View {
        CatastrophicErrorContent(
            cloudProbeState: cloudProbeState,
            onRestoreFromCloud: { recoveryConfirmation = .restore },
            onRetryCheck: retryProbe,
            onContactSupport: contactSupport,
            onWipeOnly: { recoveryConfirmation = .wipe }
        )
        .task {
            probeCloud()
        }
        .onDisappear {
            cloudProbeTask?.cancel()
            cloudProbeTask = nil
        }
        .alert(item: $recoveryConfirmation) { confirmation in
            Alert(
                title: Text(confirmation.title),
                message: Text(confirmation.message),
                primaryButton: .destructive(Text(confirmation.actionTitle)) {
                    switch confirmation {
                    case .restore:
                        onRestoreFromCloud()
                    case .wipe:
                        onWipeOnly()
                    }
                },
                secondaryButton: .cancel()
            )
        }
    }

    private func retryProbe() {
        cloudProbeTask?.cancel()
        cloudProbeState = .checking
        probeCloud()
    }

    private func probeCloud() {
        cloudProbeTask?.cancel()
        cloudProbeTask = Task.detached {
            let result = await checkCatastrophicCloudRestoreBackup(provider: .iCloudDrive)
            guard !Task.isCancelled else { return }

            await MainActor.run {
                guard !Task.isCancelled else { return }
                cloudProbeState = Self.cloudProbeState(result: result)
            }
        }
    }

    private func contactSupport() {
        if let url = URL(string: "mailto:feedback@covebitcoinwallet.com") {
            UIApplication.shared.open(url)
        }
    }

    static func cloudProbeState(result: CatastrophicCloudRestoreResult) -> CloudProbeState {
        switch result {
        case .backupFound:
            .available
        case .noBackupFound:
            .noBackup
        case let .offline(message):
            .offline(message)
        case let .inconclusive(message):
            .inconclusive(message)
        case let .unreadable(message):
            .unreadable(message)
        }
    }
}

private struct CatastrophicErrorContent: View {
    let cloudProbeState: CatastrophicErrorView.CloudProbeState
    let onRestoreFromCloud: () -> Void
    let onRetryCheck: () -> Void
    let onContactSupport: () -> Void
    let onWipeOnly: () -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                Spacer()
                    .frame(height: 16)

                CatastrophicErrorHero()

                Spacer()
                    .frame(height: 40)

                CatastrophicErrorIntroduction()

                Spacer()
                    .frame(height: 24)

                CatastrophicCloudProbeContent(cloudProbeState: cloudProbeState)

                Spacer(minLength: 26)

                CatastrophicErrorActions(
                    cloudProbeState: cloudProbeState,
                    onRestoreFromCloud: onRestoreFromCloud,
                    onRetryCheck: onRetryCheck,
                    onContactSupport: onContactSupport,
                    onWipeOnly: onWipeOnly
                )
            }
            .padding(.horizontal, 28)
            .padding(.top, 12)
            .padding(.bottom, 26)
            .frame(maxWidth: .infinity)
            .containerRelativeFrame(.vertical, alignment: .center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
    }
}

private struct CatastrophicErrorHero: View {
    var body: some View {
        ZStack {
            Circle()
                .fill(Color.red.opacity(0.12))
                .frame(width: 118, height: 118)

            Circle()
                .stroke(Color.red.opacity(0.18), lineWidth: 1)
                .frame(width: 118, height: 118)

            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 42, weight: .semibold))
                .foregroundStyle(.red)
        }
    }
}

private struct CatastrophicErrorIntroduction: View {
    var body: some View {
        VStack(spacing: 16) {
            Text("Encryption Key Error")
                .font(OnboardingRecoveryTypography.heroTitle)
                .foregroundStyle(.white)
                .multilineTextAlignment(.center)

            Text(
                "Your app's encryption key doesn't match the stored data. This is unexpected and your local wallet data on this device can’t be opened safely."
            )
            .font(OnboardingRecoveryTypography.body)
            .foregroundStyle(.coveLightGray.opacity(0.76))
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 8)
    }
}

private struct CatastrophicCloudProbeContent: View {
    let cloudProbeState: CatastrophicErrorView.CloudProbeState

    var body: some View {
        switch cloudProbeState {
        case .checking:
            VStack(spacing: 12) {
                ProgressView()
                    .tint(.white)

                Text("Checking for an available cloud backup...")
                    .font(OnboardingRecoveryTypography.body)
                    .foregroundStyle(.coveLightGray.opacity(0.66))
                    .multilineTextAlignment(.center)
            }

        case .available:
            CatastrophicCloudStatusCard(
                icon: "checkmark.circle.fill",
                color: .lightGreen,
                text: "Cloud Backup data is available for this account"
            )

        case .noBackup:
            CatastrophicCloudStatusCard(
                icon: "icloud.slash",
                color: .coveLightGray,
                text: "No cloud backup was detected for this account"
            )

        case .offline:
            CatastrophicCloudStatusCard(
                icon: "wifi.exclamationmark",
                color: .orange,
                text: "This device appears to be offline. Reconnect and try the cloud backup check again"
            )

        case .inconclusive:
            CatastrophicCloudStatusCard(
                icon: "icloud.slash",
                color: .orange,
                text: "We couldn’t confirm whether a cloud backup is available. Retry the check before restoring from cloud backup"
            )

        case .unreadable:
            CatastrophicCloudStatusCard(
                icon: "exclamationmark.triangle.fill",
                color: .orange,
                text: "Cloud backup data could not be read. Retry the check before restoring from cloud backup"
            )
        }
    }
}

private struct CatastrophicErrorActions: View {
    let cloudProbeState: CatastrophicErrorView.CloudProbeState
    let onRestoreFromCloud: () -> Void
    let onRetryCheck: () -> Void
    let onContactSupport: () -> Void
    let onWipeOnly: () -> Void

    var body: some View {
        VStack(spacing: 14) {
            if cloudProbeState.allowsRestoreAttempt {
                CatastrophicRestoreButton(action: onRestoreFromCloud)
            }

            if cloudProbeState.allowsRetry {
                Button("Retry Check", action: onRetryCheck)
                    .buttonStyle(OnboardingSecondaryButtonStyle())
            }

            Button(action: onContactSupport) {
                Label("Contact Support", systemImage: "envelope")
            }
            .buttonStyle(OnboardingSecondaryButtonStyle())

            Button("Wipe Local Data", role: .destructive, action: onWipeOnly)
                .buttonStyle(
                    OnboardingSecondaryButtonStyle(
                        backgroundColor: Color.red.opacity(0.12),
                        foregroundColor: .red.opacity(0.95),
                        borderColor: Color.red.opacity(0.22)
                    )
                )
        }
    }
}

private struct CatastrophicRestoreButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label("Restore from Cloud Backup", systemImage: "icloud.and.arrow.down")
        }
        .buttonStyle(OnboardingPrimaryButtonStyle())
    }
}

private struct CatastrophicCloudStatusCard: View {
    let icon: String
    let color: Color
    let text: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: icon)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(color)
                .padding(.top, 2)

            Text(text)
                .font(OnboardingRecoveryTypography.footnote)
                .foregroundStyle(.white.opacity(0.82))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(Color.duskBlue.opacity(0.48))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(Color.coveLightGray.opacity(0.14), lineWidth: 1)
        )
    }
}

#Preview("Catastrophic Error - Available Backup") {
    CatastrophicErrorContent(
        cloudProbeState: .available,
        onRestoreFromCloud: {},
        onRetryCheck: {},
        onContactSupport: {},
        onWipeOnly: {}
    )
}

#Preview("Catastrophic Error - Checking") {
    CatastrophicErrorContent(
        cloudProbeState: .checking,
        onRestoreFromCloud: {},
        onRetryCheck: {},
        onContactSupport: {},
        onWipeOnly: {}
    )
}
