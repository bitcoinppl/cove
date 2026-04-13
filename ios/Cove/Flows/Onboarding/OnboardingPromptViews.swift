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
    let onNewHere: () -> Void
    let onHasBitcoin: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "bitcoinsign.circle",
            title: "Do you already have Bitcoin?",
            subtitle: "We’ll tailor the setup based on where you’re starting from."
        ) {
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

struct OnboardingStorageChoiceScreen: View {
    let onExchange: () -> Void
    let onHardwareWallet: () -> Void
    let onSoftwareWallet: () -> Void
    let onBack: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "tray.full",
            title: "How do you store your Bitcoin?",
            subtitle: "Choose the option that best matches what you use today."
        ) {
            VStack(spacing: 14) {
                OnboardingChoiceCard(
                    title: "On an exchange",
                    subtitle: "Move funds into a wallet you control",
                    systemImage: "building.columns"
                ) {
                    onExchange()
                }

                OnboardingChoiceCard(
                    title: "Hardware wallet",
                    subtitle: "Import a watch-only wallet from an existing device",
                    systemImage: "shield"
                ) {
                    onHardwareWallet()
                }

                OnboardingChoiceCard(
                    title: "Software wallet",
                    subtitle: "Import recovery data from another wallet app",
                    systemImage: "iphone"
                ) {
                    onSoftwareWallet()
                }
            }

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

struct OnboardingSoftwareChoiceScreen: View {
    let onCreateWallet: () -> Void
    let onImportWallet: () -> Void
    let onBack: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "arrow.left.arrow.right.square",
            title: "What would you like to do?",
            subtitle: "Create a new wallet in Cove or import the one you already use."
        ) {
            VStack(spacing: 14) {
                OnboardingChoiceCard(
                    title: "Create a new wallet",
                    subtitle: "Generate a fresh 12-word recovery phrase",
                    systemImage: "plus.circle"
                ) {
                    onCreateWallet()
                }

                OnboardingChoiceCard(
                    title: "Import existing wallet",
                    subtitle: "Use words, QR, or a Cove backup file",
                    systemImage: "square.and.arrow.down"
                ) {
                    onImportWallet()
                }
            }

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

struct OnboardingPromptScreen<Footer: View>: View {
    let icon: String
    let title: String
    let subtitle: String
    @ViewBuilder let footer: Footer

    var body: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)

            OnboardingStatusHero(
                systemImage: icon,
                pulse: false,
                iconSize: 22
            )

            Spacer()
                .frame(height: 36)

            VStack(spacing: 12) {
                Text(title)
                    .font(.system(size: 34, weight: .semibold))
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Text(subtitle)
                    .font(.footnote)
                    .foregroundStyle(.coveLightGray.opacity(0.74))
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

                    Text(subtitle)
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.74))
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

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
    CloudCheckContent()
}
