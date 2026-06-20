import SwiftUI

struct CloudCheckContent: View {
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
                Text("Looking for iCloud backup...")
                    .font(OnboardingRecoveryTypography.compactTitle)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)

                Text("This only takes a moment")
                    .font(OnboardingRecoveryTypography.body)
                    .foregroundStyle(.coveLightGray.opacity(0.7))
                    .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 24)

            Spacer(minLength: 0)
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

            Button("Get Started", action: onContinue)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
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
    let onBack: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "icloud.slash",
            title: "No iCloud Backup Found",
            subtitle: "We couldn't find a Cove backup in iCloud for this account. You can continue without cloud restore or go back."
        ) {
            Button("Continue Without Cloud Restore", action: onContinue)
                .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Back", action: onBack)
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
    let title: Text
    let subtitle: Text
    @ViewBuilder let footer: Footer

    init(
        icon: String,
        title: Text,
        subtitle: Text,
        @ViewBuilder footer: () -> Footer
    ) {
        self.icon = icon
        self.title = title
        self.subtitle = subtitle
        self.footer = footer()
    }

    init(
        icon: String,
        title: LocalizedStringKey,
        subtitle: LocalizedStringKey,
        @ViewBuilder footer: () -> Footer
    ) {
        self.icon = icon
        self.title = Text(title)
        self.subtitle = Text(subtitle)
        self.footer = footer()
    }

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
                    title
                        .font(.system(size: 34, weight: .semibold))
                        .foregroundStyle(.white)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)

                    subtitle
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
    let title: Text
    let subtitle: Text
    let systemImage: String
    var isSelected = false
    let action: () -> Void

    init(
        title: LocalizedStringKey,
        subtitle: LocalizedStringKey,
        systemImage: String,
        isSelected: Bool = false,
        action: @escaping () -> Void
    ) {
        self.title = Text(title)
        self.subtitle = Text(subtitle)
        self.systemImage = systemImage
        self.isSelected = isSelected
        self.action = action
    }

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
                    title
                        .font(.headline)
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .fixedSize(horizontal: false, vertical: true)

                    subtitle
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
    let title: Text
    let subtitle: Text
    let systemImage: String
    let isComplete: Bool
    let actionTitle: LocalizedStringKey
    let action: () -> Void

    init(
        title: LocalizedStringKey,
        subtitle: LocalizedStringKey,
        systemImage: String,
        isComplete: Bool,
        actionTitle: LocalizedStringKey,
        action: @escaping () -> Void
    ) {
        self.title = Text(title)
        self.subtitle = Text(subtitle)
        self.systemImage = systemImage
        self.isComplete = isComplete
        self.actionTitle = actionTitle
        self.action = action
    }

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
                    title
                        .font(.headline)
                        .foregroundStyle(.white)

                    subtitle
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
    let text: Text

    init(text: String) {
        self.text = Text(verbatim: text)
    }

    init(text: LocalizedStringKey) {
        self.text = Text(text)
    }

    var body: some View {
        text
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
    CloudCheckContent()
}

#Preview("Restore Offline") {
    OnboardingRestoreOfflineScreen(onContinue: {}, onBack: {})
}
