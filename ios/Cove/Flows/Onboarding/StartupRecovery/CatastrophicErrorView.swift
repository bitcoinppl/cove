import SwiftUI

struct CatastrophicErrorView: View {
    let onRestoreFromCloud: () -> Void
    let onWipeOnly: () -> Void

    enum CloudProbeState {
        case checking
        case available
        case unavailable
        case transientError
        case corrupt
    }

    @State private var cloudProbeState: CloudProbeState = .checking
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
        cloudProbeState = .checking
        probeCloud()
    }

    private func probeCloud() {
        Task.detached {
            let cloud = CloudStorage(cloudStorage: CloudStorageAccessImpl())
            do {
                let exists = try await cloud.hasAnyCloudBackup()
                await MainActor.run {
                    cloudProbeState = exists ? .available : .unavailable
                }
            } catch let error as CloudStorageError {
                await MainActor.run {
                    switch error {
                    case .NotAvailable:
                        cloudProbeState = .transientError
                    default:
                        cloudProbeState = .corrupt
                    }
                }
            } catch {
                await MainActor.run {
                    cloudProbeState = .corrupt
                }
            }
        }
    }

    private func contactSupport() {
        if let url = URL(string: "mailto:feedback@covebitcoinwallet.com") {
            UIApplication.shared.open(url)
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
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
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

        case .unavailable:
            statusCard(
                icon: "icloud.slash",
                color: .coveLightGray,
                text: "No cloud backup was detected for this account"
            )

        case .transientError:
            statusCard(
                icon: "wifi.exclamationmark",
                color: .orange,
                text: "We couldn’t confirm cloud availability. Network conditions may be unstable, but restore may still work"
            )

        case .corrupt:
            statusCard(
                icon: "exclamationmark.triangle.fill",
                color: .orange,
                text: "Cloud backup data may be damaged, but you can still attempt a restore"
            )
        }
    }

    private var actionButtons: some View {
        VStack(spacing: 14) {
            if case .available = cloudProbeState {
                restoreButton
            }

            if case .transientError = cloudProbeState {
                restoreButton

                Button(action: onRetryCheck) {
                    Text("Retry Check")
                }
                .buttonStyle(OnboardingSecondaryButtonStyle())
            }

            if case .corrupt = cloudProbeState {
                restoreButton
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
