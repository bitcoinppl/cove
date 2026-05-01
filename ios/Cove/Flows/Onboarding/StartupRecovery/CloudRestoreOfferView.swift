import SwiftUI

@_exported import CoveCore

/// Shown after the cloud backup check finds at least one backup
struct CloudRestoreOfferView: View {
    let onRestore: () -> Void
    let onSkip: () -> Void
    var warningMessage: String? = nil
    var errorMessage: String? = nil
    var providerHint: CloudRestoreProviderHint? = nil

    var body: some View {
        VStack(spacing: 0) {
            OnboardingStepIndicator(selected: 1)
                .padding(.top, 8)

            Spacer()
                .frame(height: 42)

            heroIcon

            Spacer()
                .frame(height: 44)

            VStack(spacing: 16) {
                Text(warningMessage == nil ? "iCloud Backup Found" : "Restore from iCloud")
                    .font(OnboardingRecoveryTypography.heroTitle)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)

                Text(messageBody)
                    .font(OnboardingRecoveryTypography.body)
                    .foregroundStyle(.coveLightGray.opacity(0.76))
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 8)

            Spacer()
                .frame(height: 32)

            passkeyCard

            if let warningMessage {
                warningCard(message: warningMessage)
                    .padding(.top, 14)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }

            if let errorMessage {
                errorCard(message: errorMessage)
                    .padding(.top, 14)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }

            Spacer(minLength: 26)

            VStack(spacing: 16) {
                Button(action: onRestore) {
                    Text("Restore with Passkey")
                }
                .buttonStyle(OnboardingPrimaryButtonStyle())

                Button(action: onSkip) {
                    Text("Set Up as New")
                        .font(OnboardingRecoveryTypography.bodySemibold)
                        .foregroundStyle(Color.btnGradientLight.opacity(0.95))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 28)
        .padding(.top, 12)
        .padding(.bottom, 26)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .onboardingRecoveryBackground()
        .animation(.easeInOut(duration: 0.3), value: warningMessage)
        .animation(.easeInOut(duration: 0.3), value: errorMessage)
    }

    private var messageBody: String {
        if warningMessage == nil {
            return "A previous iCloud backup was found. Restore your wallet securely using your passkey."
        }

        return "We couldn't confirm whether an iCloud backup is available. If you're reinstalling this device, you can still try restoring with your passkey."
    }

    private var heroIcon: some View {
        ZStack {
            Circle()
                .stroke(Color.btnGradientLight.opacity(0.12), lineWidth: 1)
                .frame(width: 118, height: 118)

            Circle()
                .stroke(Color.btnGradientLight.opacity(0.18), lineWidth: 1)
                .frame(width: 86, height: 86)

            Circle()
                .stroke(Color.btnGradientLight.opacity(0.24), lineWidth: 1)
                .frame(width: 58, height: 58)

            Circle()
                .fill(Color.duskBlue.opacity(0.4))
                .frame(width: 58, height: 58)

            Circle()
                .stroke(
                    LinearGradient(
                        colors: [.btnGradientLight, .btnGradientDark],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    ),
                    lineWidth: 1.5
                )
                .frame(width: 58, height: 58)

            Image(systemName: "magnifyingglass")
                .font(.system(size: 22, weight: .semibold))
                .foregroundStyle(Color.btnGradientLight)
        }
    }

    private var passkeyCard: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Recommended")
                .font(OnboardingRecoveryTypography.captionSemibold)
                .foregroundStyle(Color.btnGradientLight.opacity(0.92))
                .frame(minWidth: 76)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(
                    Capsule()
                        .fill(Color.btnGradientLight.opacity(0.12))
                )

            HStack(spacing: 14) {
                Image(systemName: "person.badge.key")
                    .font(.system(size: 19, weight: .medium))
                    .foregroundStyle(Color.btnGradientLight)
                    .frame(width: 42, height: 42)
                    .background(
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .fill(Color.btnGradientLight.opacity(0.12))
                    )

                VStack(alignment: .leading, spacing: 4) {
                    Text("Passkey Restore")
                        .font(OnboardingRecoveryTypography.bodySemibold)
                        .foregroundStyle(.white)

                    Text(providerHint?.providerName ?? "Secured with iCloud Keychain")
                        .font(OnboardingRecoveryTypography.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.58))
                }

                Spacer()
            }

            if let providerHint {
                HStack(spacing: 12) {
                    Image(systemName: "key.fill")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Color.btnGradientLight.opacity(0.92))
                        .frame(width: 22)

                    VStack(alignment: .leading, spacing: 3) {
                        Text("Passkey provider")
                            .font(OnboardingRecoveryTypography.captionSemibold)
                            .foregroundStyle(.coveLightGray.opacity(0.58))

                        Text(providerHint.providerName)
                            .font(OnboardingRecoveryTypography.footnote)
                            .foregroundStyle(.white)

                        Text("Added \(formattedProviderDate(providerHint.registeredAt))")
                            .font(.caption)
                            .foregroundStyle(.coveLightGray.opacity(0.58))
                    }

                    Spacer()
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(Color.duskBlue.opacity(0.34))
                )
            }

            Text("Your passkey is stored securely in iCloud Keychain and syncs across all your Apple devices.")
                .font(OnboardingRecoveryTypography.subheadline)
                .foregroundStyle(.coveLightGray.opacity(0.74))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 18)
        .background(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(Color.duskBlue.opacity(0.48))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .stroke(Color.coveLightGray.opacity(0.14), lineWidth: 1)
        )
    }

    private func formattedProviderDate(_ registeredAt: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(registeredAt))
        return date.formatted(.dateTime.month(.abbreviated).day().year())
    }

    private func errorCard(message: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(.orange)
                .padding(.top, 2)

            Text(message)
                .font(OnboardingRecoveryTypography.footnote)
                .foregroundStyle(.orange.opacity(0.95))
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
                .stroke(Color.orange.opacity(0.28), lineWidth: 1)
        )
    }

    private func warningCard(message: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "wifi.exclamationmark")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(Color.btnGradientLight.opacity(0.95))
                .padding(.top, 2)

            Text(message)
                .font(OnboardingRecoveryTypography.footnote)
                .foregroundStyle(.coveLightGray.opacity(0.9))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(Color.btnGradientLight.opacity(0.08))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(Color.btnGradientLight.opacity(0.22), lineWidth: 1)
        )
    }
}

#Preview("Backup Found") {
    CloudRestoreOfferView(onRestore: {}, onSkip: {})
}

#Preview("Backup Unconfirmed") {
    CloudRestoreOfferView(
        onRestore: {},
        onSkip: {},
        warningMessage: "We couldn't confirm iCloud backup availability because connectivity or iCloud may be unavailable. You can try restore now or check Cloud Backup later in Settings."
    )
}

#Preview("Backup Found Error") {
    CloudRestoreOfferView(
        onRestore: {},
        onSkip: {},
        errorMessage: "We couldn’t verify your passkey. Try again."
    )
}
