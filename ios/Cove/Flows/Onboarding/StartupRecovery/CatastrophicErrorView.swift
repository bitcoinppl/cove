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

    @State private var cloudProbeState: CloudProbeState = .checking
    @State private var cloudProbeTask: Task<Void, Never>?
    @State private var showWipeConfirmation = false

    var body: some View {
        CatastrophicErrorContent(
            cloudProbeState: cloudProbeState,
            onRestoreFromCloud: onRestoreFromCloud,
            onRetryCheck: retryProbe,
            onContactSupport: contactSupport,
            onWipeOnly: { showWipeConfirmation = true }
        )
        .task {
            probeCloud()
        }
        .onDisappear {
            cloudProbeTask?.cancel()
            cloudProbeTask = nil
        }
        .alert("Wipe All Local Data?", isPresented: $showWipeConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Wipe Data", role: .destructive) {
                onWipeOnly()
            }
        } message: {
            Text(
                "This will permanently delete all wallet data on this device. Make sure you have your recovery phrases backed up. This cannot be undone."
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

                Spacer()
                    .frame(height: 40)

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

                Spacer()
                    .frame(height: 24)

                cloudProbeContent

                Spacer(minLength: 26)

                actionButtons
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

    @ViewBuilder
    private var cloudProbeContent: some View {
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
            statusCard(
                icon: "checkmark.circle.fill",
                color: .lightGreen,
                text: "A cloud backup is available and can be used to restore this device"
            )

        case .noBackup:
            statusCard(
                icon: "icloud.slash",
                color: .coveLightGray,
                text: "No cloud backup was detected for this account"
            )

        case .offline:
            statusCard(
                icon: "wifi.exclamationmark",
                color: .orange,
                text: "This device appears to be offline. Reconnect and try the cloud backup check again"
            )

        case .inconclusive:
            statusCard(
                icon: "icloud.slash",
                color: .orange,
                text: "We couldn’t confirm whether a cloud backup is available. Retry the check before restoring from cloud backup"
            )

        case .unreadable:
            statusCard(
                icon: "exclamationmark.triangle.fill",
                color: .orange,
                text: "Cloud backup data could not be read. Retry the check before restoring from cloud backup"
            )
        }
    }

    private var actionButtons: some View {
        VStack(spacing: 14) {
            if cloudProbeState.allowsRestoreAttempt {
                restoreButton
            }

            if cloudProbeState.allowsRetry {
                Button(action: onRetryCheck) {
                    Text("Retry Check")
                }
                .buttonStyle(OnboardingSecondaryButtonStyle())
            }

            Button(action: onContactSupport) {
                Label("Contact Support", systemImage: "envelope")
            }
            .buttonStyle(OnboardingSecondaryButtonStyle())

            Button(role: .destructive, action: onWipeOnly) {
                Text("Wipe Local Data")
            }
            .buttonStyle(
                OnboardingSecondaryButtonStyle(
                    backgroundColor: Color.red.opacity(0.12),
                    foregroundColor: .red.opacity(0.95),
                    borderColor: Color.red.opacity(0.22)
                )
            )
        }
    }

    private var restoreButton: some View {
        Button(action: onRestoreFromCloud) {
            Label("Restore from Cloud Backup", systemImage: "icloud.and.arrow.down")
        }
        .buttonStyle(OnboardingPrimaryButtonStyle())
    }

    private func statusCard(icon: String, color: Color, text: String) -> some View {
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
