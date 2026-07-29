import SwiftUI

@_exported import CoveCore

/// Shown after the cloud backup check finds at least one backup
struct CloudRestoreOfferView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    let onRestore: () -> Void
    let onSkip: () -> Void
    var warningMessage: String? = nil
    var errorMessage: String? = nil
    var providerHint: CloudRestoreProviderHint? = nil

    var body: some View {
        CloudRestoreOfferContent(
            title: warningMessage == nil ? "iCloud Backup Found" : "Restore from iCloud",
            message: messageBody,
            warningMessage: warningMessage,
            errorMessage: errorMessage,
            providerHint: providerHint,
            onRestore: onRestore,
            onSkip: onSkip
        )
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.3), value: warningMessage)
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.3), value: errorMessage)
    }

    private var messageBody: String {
        if warningMessage == nil {
            return "A previous iCloud backup was found. Restore your wallet securely using your passkey."
        }

        return "We couldn't confirm whether an iCloud backup is available. If you're reinstalling this device, you can still try restoring with your passkey."
    }
}

private struct CloudRestoreOfferContent: View {
    let title: String
    let message: String
    let warningMessage: String?
    let errorMessage: String?
    let providerHint: CloudRestoreProviderHint?
    let onRestore: () -> Void
    let onSkip: () -> Void

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                OnboardingStepIndicator(selected: 1)
                    .padding(.top, 48)

                Spacer()
                    .frame(height: 5)

                CloudRestoreHeroIcon()

                Spacer()
                    .frame(height: 16)

                VStack(spacing: 16) {
                    Text(title)
                        .font(OnboardingRecoveryTypography.heroTitle)
                        .foregroundStyle(.white)
                        .multilineTextAlignment(.center)

                    Text(message)
                        .font(OnboardingRecoveryTypography.body)
                        .foregroundStyle(.coveLightGray.opacity(0.76))
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal, 8)

                Spacer()
                    .frame(height: 28)

                CloudRestorePasskeyCard(providerHint: providerHint)

                if let warningMessage {
                    CloudRestoreMessageCard(style: .warning, message: warningMessage)
                        .padding(.top, 14)
                        .transition(.opacity.combined(with: .move(edge: .top)))
                }

                if let errorMessage {
                    CloudRestoreMessageCard(style: .error, message: errorMessage)
                        .padding(.top, 14)
                        .transition(.opacity.combined(with: .move(edge: .top)))
                }

                Spacer(minLength: 26)

                CloudRestoreOfferActions(onRestore: onRestore, onSkip: onSkip)
            }
            .padding(.horizontal, 28)
            .padding(.top, 12)
            .padding(.bottom, 26)
            .frame(maxWidth: .infinity)
        }
        .defaultScrollAnchor(.center, for: .alignment)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
    }
}

private struct CloudRestoreHeroIcon: View {
    var body: some View {
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
}

private struct CloudRestorePasskeyCard: View {
    let providerHint: CloudRestoreProviderHint?

    private var displayName: String {
        guard let providerHint else { return "Secured with your passkey provider" }

        return "Cove Cloud Backup (\(providerHint.nameSuffix))"
    }

    private var storageDescription: String {
        if let providerName = providerHint?.providerName {
            return "Your passkey is stored securely by \(providerName), and your encrypted backup is stored in iCloud."
        }

        return "Your passkey is stored securely by your passkey provider, and your encrypted backup is stored in iCloud."
    }

    var body: some View {
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

                    Text(displayName)
                        .font(OnboardingRecoveryTypography.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.58))
                }

                Spacer()
            }

            if let providerHint {
                CloudRestoreProviderDetails(providerHint: providerHint)
            }

            HStack(alignment: .center, spacing: 16) {
                Image(systemName: "lock")
                    .font(.system(size: 19, weight: .semibold))
                    .foregroundStyle(Color.btnGradientLight)
                    .frame(width: 48)

                Text(storageDescription)
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
}

private struct CloudRestoreProviderDetails: View {
    let providerHint: CloudRestoreProviderHint

    private var registeredDate: String {
        let date = Date(timeIntervalSince1970: TimeInterval(providerHint.registeredAt))
        return date.formatted(.dateTime.month(.abbreviated).day().year())
    }

    var body: some View {
        Divider()
            .overlay(Color.coveLightGray.opacity(0.16))

        VStack(alignment: .leading, spacing: 14) {
            Text("Provider Details")
                .font(OnboardingRecoveryTypography.subheadline.weight(.semibold))
                .foregroundStyle(.coveLightGray.opacity(0.72))

            if let providerName = providerHint.providerName {
                HStack(alignment: .center, spacing: 14) {
                    CloudRestoreProviderDetailItem(
                        icon: "key",
                        label: "STORED IN",
                        value: providerName
                    )

                    Rectangle()
                        .fill(Color.coveLightGray.opacity(0.14))
                        .frame(width: 1, height: 46)

                    CloudRestoreProviderDetailItem(
                        icon: "calendar",
                        label: "CREATED",
                        value: registeredDate
                    )
                }
            } else {
                CloudRestoreProviderDetailItem(
                    icon: "calendar",
                    label: "CREATED",
                    value: registeredDate
                )
            }
        }

        Divider()
            .overlay(Color.coveLightGray.opacity(0.16))
    }
}

private struct CloudRestoreProviderDetailItem: View {
    let icon: String
    let label: String
    let value: String

    var body: some View {
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
}

private struct CloudRestoreOfferActions: View {
    let onRestore: () -> Void
    let onSkip: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Button("Restore with Passkey", action: onRestore)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button(action: onSkip) {
                Text("Set Up as New")
                    .font(OnboardingRecoveryTypography.bodySemibold)
                    .foregroundStyle(Color.btnGradientLight.opacity(0.95))
            }
            .buttonStyle(.plain)
            .frame(minHeight: 44)
        }
    }
}

private struct CloudRestoreMessageCard: View {
    enum Style {
        case warning
        case error
    }

    let style: Style
    let message: String

    private var icon: String {
        switch style {
        case .warning:
            "wifi.exclamationmark"
        case .error:
            "exclamationmark.triangle.fill"
        }
    }

    private var foregroundColor: Color {
        switch style {
        case .warning:
            Color.btnGradientLight.opacity(0.95)
        case .error:
            .orange
        }
    }

    private var textColor: Color {
        switch style {
        case .warning:
            Color.coveLightGray.opacity(0.9)
        case .error:
            Color.orange.opacity(0.95)
        }
    }

    private var backgroundColor: Color {
        switch style {
        case .warning:
            Color.btnGradientLight.opacity(0.08)
        case .error:
            Color.orange.opacity(0.1)
        }
    }

    private var borderColor: Color {
        switch style {
        case .warning:
            Color.btnGradientLight.opacity(0.22)
        case .error:
            Color.orange.opacity(0.28)
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: icon)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(foregroundColor)
                .padding(.top, 2)

            Text(message)
                .font(OnboardingRecoveryTypography.footnote)
                .foregroundStyle(textColor)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(backgroundColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(borderColor, lineWidth: 1)
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
