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
        ScrollView {
            VStack(spacing: 0) {
                OnboardingStepIndicator(selected: 1)
                    .padding(.top, 48)

                Spacer()
                    .frame(height: 5)

                heroIcon

                Spacer()
                    .frame(height: 16)

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
                    .frame(height: 28)

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
            .frame(maxWidth: .infinity)
            .containerRelativeFrame(.vertical, alignment: .center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
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
                .stroke(Color.btnGradientLight.opacity(0.16), lineWidth: 1)
                .frame(width: 118, height: 118)

            Circle()
                .stroke(Color.btnGradientLight.opacity(0.26), lineWidth: 1)
                .frame(width: 86, height: 86)

            Circle()
                .stroke(Color.btnGradientLight.opacity(0.88), lineWidth: 1.5)
                .frame(width: 64, height: 64)

            Image(systemName: "cloud")
                .font(.system(size: 32, weight: .semibold))
                .foregroundStyle(Color.btnGradientLight)
        }
    }

    private var passkeyCard: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Recommended")
                .font(OnboardingRecoveryTypography.captionSemibold)
                .foregroundStyle(Color.btnGradientLight.opacity(0.92))
                .frame(minWidth: 92)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(
                    Capsule()
                        .fill(Color.btnGradientLight.opacity(0.12))
                )

            HStack(spacing: 16) {
                Image(systemName: "person.badge.key")
                    .font(.system(size: 24, weight: .medium))
                    .foregroundStyle(Color.btnGradientLight)
                    .frame(width: 48, height: 48)
                    .background(
                        RoundedRectangle(cornerRadius: 13, style: .continuous)
                            .fill(Color.btnGradientLight.opacity(0.12))
                    )

                VStack(alignment: .leading, spacing: 6) {
                    Text("Passkey Restore")
                        .font(OnboardingRecoveryTypography.bodySemibold)
                        .foregroundStyle(.white)

                    Text(providerHint.map(passkeyDisplayName) ?? "Secured with your passkey provider")
                        .font(OnboardingRecoveryTypography.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.58))
                }

                Spacer()
            }

            if let providerHint {
                Divider()
                    .overlay(Color.coveLightGray.opacity(0.16))

                VStack(alignment: .leading, spacing: 14) {
                    Text("Provider Details")
                        .font(OnboardingRecoveryTypography.subheadline.weight(.semibold))
                        .foregroundStyle(.coveLightGray.opacity(0.72))

                    if let providerName = providerHint.providerName {
                        HStack(alignment: .center, spacing: 14) {
                            providerDetailItem(
                                icon: "key",
                                label: "STORED IN",
                                value: providerName
                            )

                            Rectangle()
                                .fill(Color.coveLightGray.opacity(0.14))
                                .frame(width: 1, height: 46)

                            providerDetailItem(
                                icon: "calendar",
                                label: "CREATED",
                                value: formattedProviderDate(providerHint.registeredAt)
                            )
                        }
                    } else {
                        providerDetailItem(
                            icon: "calendar",
                            label: "CREATED",
                            value: formattedProviderDate(providerHint.registeredAt)
                        )
                    }
                }

                Divider()
                    .overlay(Color.coveLightGray.opacity(0.16))
            }

            HStack(alignment: .center, spacing: 14) {
                Image(systemName: "lock")
                    .font(.system(size: 19, weight: .semibold))
                    .foregroundStyle(Color.btnGradientLight)
                    .frame(width: 28)

                Text(passkeyStorageDescription)
                    .font(OnboardingRecoveryTypography.subheadline)
                    .foregroundStyle(.coveLightGray.opacity(0.74))
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 20)
        .background(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(Color.duskBlue.opacity(0.48))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .stroke(Color.coveLightGray.opacity(0.18), lineWidth: 1)
        )
    }

    private func providerDetailItem(icon: String, label: String, value: String) -> some View {
        HStack(alignment: .center, spacing: 10) {
            Image(systemName: icon)
                .font(.system(size: 20, weight: .semibold))
                .foregroundStyle(Color.btnGradientLight)
                .frame(width: 24)

            VStack(alignment: .leading, spacing: 8) {
                Text(label)
                    .font(OnboardingRecoveryTypography.captionSemibold)
                    .foregroundStyle(.coveLightGray.opacity(0.64))

                Text(value)
                    .font(OnboardingRecoveryTypography.footnote)
                    .foregroundStyle(.white)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var passkeyStorageDescription: String {
        if let providerName = providerHint?.providerName {
            return "Your passkey is stored securely by \(providerName), and your encrypted backup is stored in iCloud."
        }

        return "Your passkey is stored securely by your passkey provider, and your encrypted backup is stored in iCloud."
    }

    private func passkeyDisplayName(_ providerHint: CloudRestoreProviderHint) -> String {
        "Cove Cloud Backup (\(providerHint.nameSuffix))"
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

#Preview("Backup Found Provider Hint") {
    CloudRestoreOfferView(
        onRestore: {},
        onSkip: {},
        providerHint: CloudRestoreProviderHint(
            providerName: "Apple Passwords",
            registeredAt: 1_777_612_800,
            nameSuffix: "09IX"
        )
    )
}

#Preview("Backup Found Provider Date") {
    CloudRestoreOfferView(
        onRestore: {},
        onSkip: {},
        providerHint: CloudRestoreProviderHint(
            providerName: nil,
            registeredAt: 1_777_612_800,
            nameSuffix: "09IY"
        )
    )
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
