import CoreImage.CIFilterBuiltins
import SwiftUI
import UIKit

struct OnboardingCreatingWalletView: View {
    let onContinue: () -> Void
    @State private var didAdvance = false

    var body: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)

            OnboardingStatusHero(
                systemImage: "wallet.bifold",
                pulse: true,
                iconSize: 22
            )

            Spacer()
                .frame(height: 40)

            VStack(spacing: 12) {
                Text("Creating your wallet")
                    .font(OnboardingRecoveryTypography.compactTitle)
                    .foregroundStyle(.white)

                Text("Generating keys and preparing your backup flow")
                    .font(OnboardingRecoveryTypography.body)
                    .foregroundStyle(.coveLightGray.opacity(0.72))
                    .multilineTextAlignment(.center)

                ProgressView()
                    .tint(.white)
                    .padding(.top, 8)
            }
            .padding(.horizontal, 24)

            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
        .task {
            guard !didAdvance else { return }

            do {
                try await Task.sleep(for: .milliseconds(900))
            } catch is CancellationError {
                return
            } catch {
                return
            }

            guard !didAdvance else { return }
            didAdvance = true
            onContinue()
        }
    }
}

struct OnboardingBackupWalletView: View {
    let branch: OnboardingBranch?
    let secretWordsSaved: Bool
    let cloudBackupEnabled: Bool
    let wordCount: Int
    let onShowWords: () -> Void
    let onEnableCloudBackup: () -> Void
    let onContinue: () -> Void

    private var canContinue: Bool {
        secretWordsSaved || cloudBackupEnabled
    }

    private var title: String {
        branch == .exchange ? "Back up your wallet before funding it" : "Back up your wallet"
    }

    private var subtitle: String {
        if branch == .exchange {
            return "You’ll fund this wallet next. Save your recovery words or enable Cloud Backup first."
        }

        return "Choose at least one backup method before continuing."
    }

    var body: some View {
        OnboardingPromptScreen(
            icon: "lock.doc",
            title: title,
            subtitle: subtitle
        ) {
            VStack(spacing: 14) {
                OnboardingStatusCard(
                    title: "Save recovery words",
                    subtitle: "Write down your \(wordCount)-word recovery phrase offline",
                    systemImage: "doc.text",
                    isComplete: secretWordsSaved,
                    actionTitle: secretWordsSaved ? "Saved" : "Show Words",
                    action: onShowWords
                )

                OnboardingStatusCard(
                    title: "Enable Cloud Backup",
                    subtitle: "Encrypt and store a backup in iCloud protected by your passkey",
                    systemImage: "icloud.and.arrow.up",
                    isComplete: cloudBackupEnabled,
                    actionTitle: cloudBackupEnabled ? "Enabled" : "Enable",
                    action: onEnableCloudBackup
                )
            }

            Button("Continue", action: onContinue)
                .buttonStyle(OnboardingPrimaryButtonStyle())
                .disabled(!canContinue)
        }
    }
}

struct OnboardingSecretWordsView: View {
    let words: [String]
    let onBack: () -> Void
    let onSaved: () -> Void

    private let columns = Array(repeating: GridItem(.flexible(), spacing: 12), count: 2)

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button("Back", action: onBack)
                    .foregroundStyle(.white)
                    .font(.headline)
                Spacer()
            }
            .padding(.horizontal, 24)
            .padding(.top, 20)

            ScrollView {
                VStack(spacing: 24) {
                    VStack(spacing: 12) {
                        Text("Your Recovery Words")
                            .font(.system(size: 34, weight: .semibold))
                            .foregroundStyle(.white)
                            .frame(maxWidth: .infinity, alignment: .leading)

                        Text("Write these down exactly in order and keep them offline. Anyone with these words can control your Bitcoin.")
                            .font(.footnote)
                            .foregroundStyle(.coveLightGray.opacity(0.74))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }

                    LazyVGrid(columns: columns, spacing: 12) {
                        ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                            OnboardingWordCard(index: index + 1, word: word)
                        }
                    }
                }
                .padding(.horizontal, 24)
                .padding(.top, 32)
                .padding(.bottom, 120)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
        .safeAreaInset(edge: .bottom) {
            Button("I Saved These Words", action: onSaved)
                .buttonStyle(OnboardingPrimaryButtonStyle())
                .padding(.horizontal, 24)
                .padding(.top, 12)
                .padding(.bottom, 24)
                .background(.clear)
        }
    }
}

struct OnboardingCloudBackupStepView: View {
    let branch: OnboardingBranch?
    let onEnabled: () -> Void
    let onSkip: () -> Void

    var body: some View {
        switch branch {
        case .softwareImport:
            OnboardingSoftwareImportCloudBackupStepView(
                onEnabled: onEnabled,
                onSkip: onSkip
            )

        case .hardware:
            OnboardingHardwareImportCloudBackupStepView(
                onEnabled: onEnabled,
                onSkip: onSkip
            )

        case .newUser, .exchange, .softwareCreate, nil:
            OnboardingCloudBackupDetailsStepView(
                onEnabled: onEnabled,
                onSkip: onSkip,
                context: .standard
            )
        }
    }
}

private struct OnboardingSoftwareImportCloudBackupStepView: View {
    @State private var showingDetails = false

    let onEnabled: () -> Void
    let onSkip: () -> Void

    var body: some View {
        if showingDetails {
            OnboardingCloudBackupDetailsStepView(
                onEnabled: onEnabled,
                onSkip: { showingDetails = false },
                context: .standard
            )
        } else {
            OnboardingSoftwareImportCloudBackupChoiceView(
                onEnable: { showingDetails = true },
                onSkip: onSkip
            )
        }
    }
}

private struct OnboardingHardwareImportCloudBackupStepView: View {
    @State private var showingDetails = false

    let onEnabled: () -> Void
    let onSkip: () -> Void

    var body: some View {
        if showingDetails {
            OnboardingCloudBackupDetailsStepView(
                onEnabled: onEnabled,
                onSkip: { showingDetails = false },
                context: .hardwareImport
            )
        } else {
            OnboardingHardwareImportCloudBackupChoiceView(
                onEnable: { showingDetails = true },
                onSkip: onSkip
            )
        }
    }
}

private struct OnboardingCloudBackupDetailsStepView: View {
    @State private var backupManager = CloudBackupManager.shared
    @State private var didComplete = false
    @State private var isStartingEnable = false

    let onEnabled: () -> Void
    let onSkip: () -> Void
    let context: CloudBackupEnableOnboardingContext

    private var onboardingMessage: String? {
        switch backupManager.status {
        case .unsupportedPasskeyProvider:
            "This passkey provider did not confirm PRF support for Cloud Backup. Try Apple Passwords (iCloud Keychain) or another supported provider such as 1Password"
        case let .error(message):
            message
        default:
            nil
        }
    }

    private var isBusy: Bool {
        isStartingEnable || {
            if case .enabling = backupManager.status { true } else { false }
        }()
    }

    var body: some View {
        ZStack {
            CloudBackupEnableOnboardingView(
                onEnable: {
                    guard !isBusy else { return }
                    isStartingEnable = true
                    backupManager.dispatch(action: .enableCloudBackupNoDiscovery)
                },
                onCancel: onSkip,
                message: onboardingMessage,
                isBusy: isBusy,
                context: context
            )

            if isBusy {
                Color.black.opacity(0.55)
                    .ignoresSafeArea()

                VStack(spacing: 14) {
                    ProgressView()
                        .tint(.white)
                    Text("Waiting for your new passkey to become available...")
                        .font(.headline)
                        .foregroundStyle(.white)
                        .multilineTextAlignment(.center)
                    Text("Cloud Backup will continue automatically")
                        .font(.subheadline)
                        .foregroundStyle(.coveLightGray)
                        .multilineTextAlignment(.center)
                }
                .padding(.horizontal, 24)
                .padding(.vertical, 20)
                .frame(maxWidth: 320)
                .background(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(Color.midnightBlue.opacity(0.96))
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .stroke(Color.white.opacity(0.08), lineWidth: 1)
                )
                .shadow(color: .black.opacity(0.35), radius: 20, y: 10)
            }
        }
        .task {
            completeIfEnabled()
        }
        .onChange(of: backupManager.status, initial: true) { _, status in
            if case .enabling = status {
                isStartingEnable = false
            } else if isStartingEnable {
                isStartingEnable = false
            }
            completeIfEnabled(status: status)
        }
        .onChange(of: backupManager.isConfigured) { _, _ in
            completeIfEnabled()
        }
    }

    private func completeIfEnabled(status: CloudBackupStatus? = nil) {
        guard !didComplete else { return }
        let currentStatus = status ?? backupManager.status
        let isEnabled = if case .enabled = currentStatus {
            true
        } else {
            backupManager.isCloudBackupEnabled
        }
        guard isEnabled else { return }
        didComplete = true
        onEnabled()
    }
}

private struct OnboardingSoftwareImportCloudBackupChoiceView: View {
    let onEnable: () -> Void
    let onSkip: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "icloud.and.arrow.up",
            title: "Protect this wallet with Cloud Backup?",
            subtitle: "Cloud Backup makes it easier to recover this wallet if you lose this device."
        ) {
            VStack(spacing: 14) {
                VStack(alignment: .leading, spacing: 14) {
                    Text("Your wallet backup is end-to-end encrypted before it leaves your device, stored in iCloud, and locked with a passkey only you control.")
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.78))
                        .fixedSize(horizontal: false, vertical: true)

                    Text("You can skip this now and enable it later from Settings.")
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.64))
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(18)
                .background(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(Color.duskBlue.opacity(0.5))
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .stroke(Color.coveLightGray.opacity(0.14), lineWidth: 1)
                )

                Button("Enable Cloud Backup", action: onEnable)
                    .buttonStyle(OnboardingPrimaryButtonStyle())

                Button("Not Now", action: onSkip)
                    .buttonStyle(OnboardingSecondaryButtonStyle())
            }
        }
    }
}

private struct OnboardingHardwareImportCloudBackupChoiceView: View {
    let onEnable: () -> Void
    let onSkip: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "icloud.and.arrow.up",
            title: "Protect this hardware wallet with Cloud Backup?",
            subtitle: "Cloud Backup makes it easier to restore this wallet's configuration and labels if you lose this device."
        ) {
            VStack(spacing: 14) {
                VStack(alignment: .leading, spacing: 14) {
                    Text("This backs up the imported hardware wallet configuration and labels stored in Cove so you can restore this wallet view later.")
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.78))
                        .fixedSize(horizontal: false, vertical: true)

                    Text("Enabling this also turns on Cloud Backup for Cove more broadly, so compatible wallets you create later, as well as wallet labels, will be backed up.")
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.72))
                        .fixedSize(horizontal: false, vertical: true)

                    Text("This does not back up your hardware wallet seed or private keys.")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.white.opacity(0.86))
                        .fixedSize(horizontal: false, vertical: true)

                    Text("You can skip this now and enable it later from Settings.")
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.64))
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(18)
                .background(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(Color.duskBlue.opacity(0.5))
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .stroke(Color.coveLightGray.opacity(0.14), lineWidth: 1)
                )

                Button("Enable Cloud Backup", action: onEnable)
                    .buttonStyle(OnboardingPrimaryButtonStyle())

                Button("Not Now", action: onSkip)
                    .buttonStyle(OnboardingSecondaryButtonStyle())
            }
        }
    }
}

struct OnboardingExchangeFundingView: View {
    @Environment(AppManager.self) private var app

    let walletId: WalletId?
    let onContinue: () -> Void

    @State private var walletManager: WalletManager?
    @State private var addressInfo: AddressInfo?
    @State private var errorMessage: String?
    private let pasteboard = UIPasteboard.general

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 24) {
                    VStack(spacing: 12) {
                        Text("Your wallet is ready to fund")
                            .font(.system(size: 34, weight: .semibold))
                            .foregroundStyle(.white)
                            .frame(maxWidth: .infinity, alignment: .leading)

                        Text("Move your Bitcoin off the exchange and into the wallet you now control.")
                            .font(.footnote)
                            .foregroundStyle(.coveLightGray.opacity(0.74))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }

                    if let errorMessage {
                        OnboardingInlineMessage(text: errorMessage)
                    } else if let addressInfo {
                        VStack(spacing: 18) {
                            OnboardingAddressQr(address: addressInfo.addressUnformatted())

                            VStack(alignment: .leading, spacing: 8) {
                                Text("Deposit address")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.coveLightGray.opacity(0.72))

                                Text(addressInfo.addressUnformatted().addressSpacedOut())
                                    .font(.system(.body, design: .monospaced))
                                    .foregroundStyle(.white)
                                    .textSelection(.enabled)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(18)
                            .background(
                                RoundedRectangle(cornerRadius: 16, style: .continuous)
                                    .fill(Color.duskBlue.opacity(0.48))
                            )
                            .overlay(
                                RoundedRectangle(cornerRadius: 16, style: .continuous)
                                    .stroke(Color.coveLightGray.opacity(0.15), lineWidth: 1)
                            )

                            Button("Copy Address") {
                                pasteboard.string = addressInfo.addressUnformatted()
                            }
                            .buttonStyle(OnboardingSecondaryButtonStyle())
                        }
                    } else {
                        VStack(spacing: 12) {
                            ProgressView()
                                .tint(.white)
                            Text("Loading deposit address")
                                .font(.body)
                                .foregroundStyle(.white)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 48)
                    }
                }
                .padding(.horizontal, 24)
                .padding(.top, 32)
            }

            VStack(spacing: 14) {
                Button("Continue", action: onContinue)
                    .buttonStyle(OnboardingPrimaryButtonStyle())
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 24)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
        .task {
            await loadAddress()
        }
    }

    private func loadAddress() async {
        guard addressInfo == nil else { return }
        guard let walletId else {
            errorMessage = "The new wallet could not be loaded."
            return
        }

        do {
            let manager = try app.getWalletManager(id: walletId)
            let address = try await manager.firstAddress()
            await MainActor.run {
                walletManager = manager
                addressInfo = address
            }
        } catch {
            await MainActor.run {
                errorMessage = error.localizedDescription
            }
        }
    }
}

struct OnboardingWordCard: View {
    let index: Int
    let word: String

    var body: some View {
        HStack(spacing: 10) {
            Text("\(index)")
                .font(.caption.weight(.semibold))
                .foregroundStyle(Color.btnGradientLight)
                .frame(width: 24)

            Text(word)
                .font(.system(.callout, design: .monospaced).weight(.medium))
                .foregroundStyle(.white)

            Spacer()
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color.duskBlue.opacity(0.5))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.coveLightGray.opacity(0.15), lineWidth: 1)
        )
    }
}

struct OnboardingAddressQr: View {
    let address: String

    private func generateQr(from string: String) -> UIImage {
        let data = Data(string.utf8)
        let filter = CIFilter.qrCodeGenerator()
        filter.setValue(data, forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")

        let transform = CGAffineTransform(scaleX: 10, y: 10)
        let context = CIContext()

        guard let outputImage = filter.outputImage?.transformed(by: transform),
              let cgImage = context.createCGImage(outputImage, from: outputImage.extent)
        else {
            return UIImage(systemName: "xmark.circle") ?? UIImage()
        }

        return UIImage(cgImage: cgImage)
    }

    var body: some View {
        Image(uiImage: generateQr(from: address))
            .interpolation(.none)
            .resizable()
            .scaledToFit()
            .padding(12)
            .background(Color.white)
            .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            .frame(maxWidth: 320)
            .frame(maxWidth: .infinity)
    }
}

struct OnboardingErrorScreen: View {
    let title: String
    let message: String

    var body: some View {
        OnboardingPromptScreen(
            icon: "exclamationmark.triangle",
            title: title,
            subtitle: message
        ) {
            EmptyView()
        }
    }
}
