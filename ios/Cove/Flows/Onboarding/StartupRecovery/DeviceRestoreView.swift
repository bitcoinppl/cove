import SwiftUI

@_exported import CoveCore

struct DeviceRestoreView: View {
    let restoreState: OnboardingRestoreState
    let onDone: () -> Void
    let onRetry: () -> Void
    let onContinueWithoutBackup: () -> Void

    private var combinedRestoreProgress: Double {
        guard case let .restoring(flow) = restoreState else { return 0 }

        switch flow {
        case .finding:
            return 0

        case let .downloading(completed, total):
            guard total > 0 else { return 0 }
            let totalWork = Double(total) * 2
            return Double(completed) / totalWork

        case let .restoring(completed, total):
            guard total > 0 else { return 0 }
            let totalWork = Double(total) * 2
            return Double(total + completed) / totalWork
        }
    }

    var body: some View {
        DeviceRestoreContent(
            restoreState: restoreState,
            combinedProgress: combinedRestoreProgress,
            onDone: onDone,
            onRetry: onRetry,
            onContinueWithoutBackup: onContinueWithoutBackup
        )
    }
}

private struct DeviceRestoreContent: View {
    let restoreState: OnboardingRestoreState
    let combinedProgress: Double
    let onDone: () -> Void
    let onRetry: () -> Void
    let onContinueWithoutBackup: () -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                Spacer(minLength: 0)

                DeviceRestoreHero(restoreState: restoreState)

                Spacer()
                    .frame(height: 44)

                DeviceRestoreTitle(restoreState: restoreState)

                if isRestoring {
                    Spacer()
                        .frame(height: 18)

                    OnboardingThinProgressBar(progress: combinedProgress)
                }

                Spacer(minLength: 28)

                DeviceRestoreBottomContent(
                    restoreState: restoreState,
                    onDone: onDone,
                    onRetry: onRetry,
                    onContinueWithoutBackup: onContinueWithoutBackup
                )
            }
            .padding(.horizontal, 28)
            .padding(.top, 18)
            .padding(.bottom, 28)
            .frame(maxWidth: .infinity)
            .containerRelativeFrame(.vertical, alignment: .center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
    }

    private var isRestoring: Bool {
        switch restoreState {
        case .idle, .restoring:
            true
        case .complete, .failed:
            false
        }
    }
}

private struct DeviceRestoreHero: View {
    let restoreState: OnboardingRestoreState

    var body: some View {
        switch restoreState {
        case .idle:
            OnboardingStatusHero(
                systemImage: "icloud.and.arrow.down",
                pulse: false,
                iconSize: 22
            )

        case .restoring:
            OnboardingStatusHero(
                systemImage: "icloud.and.arrow.down",
                pulse: true,
                iconSize: 22
            )

        case .complete:
            OnboardingStatusHero(
                systemImage: "checkmark",
                tint: .lightGreen,
                fillColor: Color.lightGreen.opacity(0.12),
                iconSize: 26
            )

        case .failed:
            ZStack {
                Circle()
                    .fill(Color.red.opacity(0.12))
                    .frame(width: 118, height: 118)

                Circle()
                    .stroke(Color.red.opacity(0.2), lineWidth: 1)
                    .frame(width: 118, height: 118)

                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 40, weight: .semibold))
                    .foregroundStyle(.red)
            }
        }
    }
}

private struct DeviceRestoreTitle: View {
    let restoreState: OnboardingRestoreState

    var body: some View {
        switch restoreState {
        case .idle, .restoring:
            DeviceRestoreProgressTitle()

        case let .complete(report):
            DeviceRestoreCompleteTitle(failedCount: Int(report.walletsFailed))

        case .failed:
            DeviceRestoreFailedTitle()
        }
    }
}

private struct DeviceRestoreProgressTitle: View {
    var body: some View {
        VStack(spacing: 10) {
            Text("Restoring from iCloud...")
                .font(OnboardingRecoveryTypography.compactTitle)
                .foregroundStyle(.white)
                .multilineTextAlignment(.center)

            Text("This might take a few minutes")
                .font(OnboardingRecoveryTypography.body)
                .foregroundStyle(.coveLightGray.opacity(0.7))
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 12)
    }
}

private struct DeviceRestoreCompleteTitle: View {
    let failedCount: Int

    var body: some View {
        VStack(spacing: 10) {
            Text(failedCount == 0 ? "You’re all set" : "Some wallets were restored")
                .font(OnboardingRecoveryTypography.compactTitle)
                .foregroundStyle(.white)
                .multilineTextAlignment(.center)

            Text(
                failedCount == 0
                    ? "Your wallets have been restored."
                    : "^[\(failedCount) wallet](inflect: true) could not be restored. You can retry from backup settings."
            )
            .font(OnboardingRecoveryTypography.body)
            .foregroundStyle(.coveLightGray.opacity(0.7))
            .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 12)
    }
}

private struct DeviceRestoreFailedTitle: View {
    var body: some View {
        VStack(spacing: 12) {
            Text("Restore Failed")
                .font(OnboardingRecoveryTypography.heroTitle)
                .foregroundStyle(.white)
                .multilineTextAlignment(.center)

            Text("Something went wrong while restoring your wallets")
                .font(OnboardingRecoveryTypography.body)
                .foregroundStyle(.coveLightGray.opacity(0.76))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 8)
    }
}

private struct DeviceRestoreBottomContent: View {
    let restoreState: OnboardingRestoreState
    let onDone: () -> Void
    let onRetry: () -> Void
    let onContinueWithoutBackup: () -> Void

    var body: some View {
        switch restoreState {
        case .idle, .restoring:
            EmptyView()

        case let .complete(report):
            DeviceRestoreCompleteActions(report: report, onDone: onDone)

        case let .failed(message):
            DeviceRestoreFailedActions(
                message: message,
                onRetry: onRetry,
                onContinueWithoutBackup: onContinueWithoutBackup
            )
        }
    }
}

private struct DeviceRestoreCompleteActions: View {
    let report: CloudBackupRestoreReport
    let onDone: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            if report.walletsFailed > 0 {
                DeviceRestoreWarningCard(
                    message: "\(report.walletsFailed) wallet(s) could not be restored"
                )
            }

            if !report.labelsFailedWalletNames.isEmpty {
                DeviceRestoreWarningCard(
                    message: "\(report.labelsFailedWalletNames.count) restored wallet(s) had labels that could not be imported"
                )
            }

            Button("Done", action: onDone)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

private struct DeviceRestoreFailedActions: View {
    let message: String
    let onRetry: () -> Void
    let onContinueWithoutBackup: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            DeviceRestoreWarningCard(message: message)

            Button("Retry", action: onRetry)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Continue without backup", action: onContinueWithoutBackup)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

private struct DeviceRestoreWarningCard: View {
    let message: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(.orange)
                .padding(.top, 2)

            Text(message)
                .font(OnboardingRecoveryTypography.footnote)
                .foregroundStyle(.orange.opacity(0.92))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(Color.orange.opacity(0.1))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(Color.orange.opacity(0.3), lineWidth: 1)
        )
    }
}

#Preview("Restore Progress") {
    DeviceRestoreContent(
        restoreState: .restoring(.finding),
        combinedProgress: 0.25,
        onDone: {},
        onRetry: {},
        onContinueWithoutBackup: {}
    )
}

#Preview("Restore Success") {
    DeviceRestoreContent(
        restoreState: .complete(
            CloudBackupRestoreReport(
                walletsRestored: 4,
                walletsFailed: 0,
                failedWalletErrors: [],
                labelsFailedWalletNames: [],
                labelsFailedErrors: []
            )
        ),
        combinedProgress: 1,
        onDone: {},
        onRetry: {},
        onContinueWithoutBackup: {}
    )
}

#Preview("Restore Partial Success") {
    DeviceRestoreContent(
        restoreState: .complete(
            CloudBackupRestoreReport(
                walletsRestored: 3,
                walletsFailed: 1,
                failedWalletErrors: ["Wallet 4 failed to restore"],
                labelsFailedWalletNames: ["Wallet 2"],
                labelsFailedErrors: ["Failed to parse labels: invalid type"]
            )
        ),
        combinedProgress: 1,
        onDone: {},
        onRetry: {},
        onContinueWithoutBackup: {}
    )
}

#Preview("Restore Error") {
    DeviceRestoreContent(
        restoreState: .failed(message: "Restore timed out. Please try again."),
        combinedProgress: 0,
        onDone: {},
        onRetry: {},
        onContinueWithoutBackup: {}
    )
}
