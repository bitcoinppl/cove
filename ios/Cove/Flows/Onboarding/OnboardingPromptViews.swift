import SwiftUI

struct CloudCheckContent: View {
    let onContinue: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)

            OnboardingStatusHero(
                systemImage: "icloud",
                pulse: true,
                iconSize: 22
            )

            Spacer()
                .frame(height: 44)

            VStack(spacing: 10) {
                Text("Looking for your iCloud backup")
                    .font(OnboardingRecoveryTypography.compactTitle)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)

                Text("iCloud can take a little while on a newly set-up iPhone. Cove will keep checking.")
                    .font(OnboardingRecoveryTypography.body)
                    .foregroundStyle(.coveLightGray.opacity(0.7))
                    .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 24)

            Spacer(minLength: 0)

            Button("Continue Setup", action: onContinue)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
        .padding(.horizontal, 28)
        .padding(.top, 18)
        .padding(.bottom, 28)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
    }
}

struct OnboardingWelcomeScreen: View {
    let errorMessage: String?
    let cloudRestoreState: OnboardingCloudRestoreState
    let onRestoreFromCoveBackup: () -> Void
    let onContinue: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "sparkles",
            title: "Welcome to Cove",
            subtitle: "A self-custody Bitcoin wallet focused on secure backups, clear flows, and hardware wallet support."
        ) {
            if let errorMessage {
                OnboardingInlineMessage(text: errorMessage)
            }

            if cloudRestoreState == .checking {
                OnboardingCloudCheckStatus(
                    text: "Checking iCloud for an existing Cove backup…"
                )
            }

            Button("Get Started", action: onContinue)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Restore from Cove Backup", action: onRestoreFromCoveBackup)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

struct OnboardingCloudCheckStatus: View {
    let text: String

    var body: some View {
        HStack(spacing: 12) {
            ProgressView()
                .tint(.white)

            Text(text)
                .font(.footnote)
                .foregroundStyle(.white.opacity(0.84))
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color.duskBlue.opacity(0.56))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.coveLightGray.opacity(0.16), lineWidth: 1)
        )
    }
}

struct OnboardingBitcoinChoiceScreen: View {
    let errorMessage: String?
    let onNewHere: () -> Void
    let onHasBitcoin: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "bitcoinsign.circle",
            title: "Do you already have Bitcoin?",
            subtitle: "We’ll tailor the setup based on where you’re starting from."
        ) {
            if let errorMessage {
                OnboardingInlineMessage(text: errorMessage)
            }

            VStack(spacing: 14) {
                OnboardingChoiceCard(
                    title: "No, I’m new here",
                    subtitle: "Create a new wallet and learn the basics",
                    systemImage: "leaf"
                ) {
                    onNewHere()
                }

                OnboardingChoiceCard(
                    title: "Yes, I have Bitcoin",
                    subtitle: "Import or connect the wallet you already use",
                    systemImage: "arrow.trianglehead.branch"
                ) {
                    onHasBitcoin()
                }
            }
        }
    }
}

struct OnboardingRestoreUnavailableScreen: View {
    let onContinue: () -> Void
    let onCheckAgain: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "icloud.slash",
            title: "No Backup Found Yet",
            subtitle: "iCloud may still be syncing on this iPhone. Check again in a moment, or continue setup and check Cloud Backup from Settings."
        ) {
            Button("Check Again", action: onCheckAgain)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Continue Setup", action: onContinue)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

struct OnboardingRestoreOfflineScreen: View {
    let onContinue: () -> Void
    let onBack: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "wifi.slash",
            title: "You’re Offline",
            subtitle: "Cove can’t check for an iCloud backup right now. You can continue onboarding and check Cloud Backup later in Settings."
        ) {
            Button("Continue Without Cloud Restore", action: onContinue)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

struct OnboardingStorageChoiceScreen: View {
    let errorMessage: String?
    let onRestoreFromCoveBackup: (() -> Void)?
    let onSelectStorage: (OnboardingStorageSelection) -> Void
    let onBack: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "tray.full",
            title: "How do you store your Bitcoin?",
            subtitle: "Choose the option that best matches what you use today."
        ) {
            if let errorMessage {
                OnboardingInlineMessage(text: errorMessage)
            }

            VStack(spacing: 14) {
                if let onRestoreFromCoveBackup {
                    OnboardingCloudRestoreChoiceCard(action: onRestoreFromCoveBackup)
                }

                OnboardingChoiceCard(
                    title: "On an exchange",
                    subtitle: "Move funds into a wallet you control",
                    systemImage: "building.columns"
                ) {
                    onSelectStorage(.exchange)
                }

                OnboardingChoiceCard(
                    title: "Hardware wallet",
                    subtitle: "Import a watch-only wallet from an existing device",
                    systemImage: "shield"
                ) {
                    onSelectStorage(.hardwareWallet)
                }

                OnboardingChoiceCard(
                    title: "Software wallet",
                    subtitle: "Import recovery data from another wallet app",
                    systemImage: "iphone"
                ) {
                    onSelectStorage(.softwareWallet)
                }
            }

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

struct OnboardingCloudRestoreChoiceCard: View {
    let action: () -> Void

    var body: some View {
        OnboardingChoiceCard(
            title: "Restore from Cove backup",
            subtitle: "Use your passkey to restore from iCloud",
            systemImage: "icloud.and.arrow.down",
            action: action
        )
    }
}

struct OnboardingPromptScreen<Footer: View>: View {
    let icon: String
    let title: String
    let subtitle: String
    @ViewBuilder let footer: Footer

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                Spacer(minLength: 0)

                OnboardingStatusHero(
                    systemImage: icon,
                    pulse: true,
                    iconSize: 22
                )

                Spacer()
                    .frame(height: 36)

                VStack(spacing: 12) {
                    Text(title)
                        .font(.system(size: 34, weight: .semibold))
                        .foregroundStyle(.white)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)

                    Text(subtitle)
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.74))
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.horizontal, 24)

                Spacer()
                    .frame(height: 26)

                VStack(spacing: 14) {
                    footer
                }
                .padding(.horizontal, 24)

                Spacer(minLength: 0)
            }
            .padding(.vertical, 24)
            .frame(maxWidth: .infinity)
            .containerRelativeFrame(.vertical, alignment: .center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
    }
}

struct OnboardingChoiceCard: View {
    let title: String
    let subtitle: String
    let systemImage: String
    var isSelected = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 14) {
                ZStack {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(Color.btnGradientLight.opacity(0.18))
                        .frame(width: 48, height: 48)

                    Image(systemName: systemImage)
                        .font(.system(size: 19, weight: .semibold))
                        .foregroundStyle(Color.btnGradientLight)
                }

                VStack(alignment: .leading, spacing: 6) {
                    Text(title)
                        .font(.headline)
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .fixedSize(horizontal: false, vertical: true)

                    Text(subtitle)
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.74))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .layoutPriority(1)

                Image(systemName: isSelected ? "checkmark.circle.fill" : "chevron.right")
                    .font(.system(size: isSelected ? 18 : 14, weight: .semibold))
                    .foregroundStyle(isSelected ? Color.btnGradientLight : .white.opacity(0.46))
            }
            .padding(18)
            .background(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(Color.duskBlue.opacity(0.5))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(Color.coveLightGray.opacity(0.14), lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }
}

struct OnboardingStatusCard: View {
    let title: String
    let subtitle: String
    let systemImage: String
    let isComplete: Bool
    let actionTitle: String
    let action: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(Color.btnGradientLight)
                    .frame(width: 40, height: 40)
                    .background(Color.btnGradientLight.opacity(0.16))
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                VStack(alignment: .leading, spacing: 4) {
                    Text(title)
                        .font(.headline)
                        .foregroundStyle(.white)

                    Text(subtitle)
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.74))
                }

                Spacer()

                if isComplete {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 20, weight: .semibold))
                        .foregroundStyle(Color.lightGreen)
                }
            }

            Button(actionTitle, action: action)
                .buttonStyle(
                    isComplete
                        ? OnboardingSecondaryButtonStyle(
                            backgroundColor: .duskBlue.opacity(0.75),
                            foregroundColor: .white.opacity(0.84),
                            borderColor: .coveLightGray.opacity(0.14)
                        )
                        : OnboardingSecondaryButtonStyle()
                )
        }
        .padding(18)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(Color.duskBlue.opacity(0.5))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(Color.coveLightGray.opacity(0.14), lineWidth: 1)
        )
    }
}

struct OnboardingInlineMessage: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(Color.red.opacity(0.2))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(Color.red.opacity(0.35), lineWidth: 1)
            )
    }
}

#Preview("Cloud Check") {
    CloudCheckContent(onContinue: {})
}

#Preview("Restore Offline") {
    OnboardingRestoreOfflineScreen(onContinue: {}, onBack: {})
}
