import CoreImage.CIFilterBuiltins
import SwiftUI
import UIKit
import UniformTypeIdentifiers

@_exported import CoveCore

extension WeakReconciler: OnboardingManagerReconciler where Reconciler == OnboardingManager {}

@Observable
final class OnboardingManager: AnyReconciler, OnboardingManagerReconciler, @unchecked Sendable {
    @ObservationIgnored let rust: RustOnboardingManager
    @ObservationIgnored private let rustBridge = DispatchQueue(
        label: "cove.onboarding.rustbridge", qos: .userInitiated
    )
    let app: AppManager
    var state: OnboardingState
    var isComplete = false
    var cloudCheckWarning: String?

    typealias Message = OnboardingReconcileMessage

    init(app: AppManager) {
        self.app = app
        let rust = RustOnboardingManager()
        self.rust = rust
        self.state = rust.state()
        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    func dispatch(_ action: OnboardingAction) {
        rustBridge.async { [rust] in
            rust.dispatch(action: action)
        }
    }

    func reconcile(message: OnboardingReconcileMessage) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            switch message {
            case let .step(step):
                applyStep(step)
            case let .branch(branch):
                state.branch = branch
            case let .hardwareDevice(device):
                state.hardwareDevice = device
            case let .createdWords(words):
                state.createdWords = words
            case let .cloudBackupEnabled(enabled):
                state.cloudBackupEnabled = enabled
            case let .secretWordsSaved(saved):
                state.secretWordsSaved = saved
            case let .errorMessageChanged(errorMessage):
                state.errorMessage = errorMessage
            case .complete:
                isComplete = true
            }
        }
    }

    func reconcileMany(messages: [OnboardingReconcileMessage]) {
        messages.forEach { reconcile(message: $0) }
    }

    private func applyStep(_ step: OnboardingStep) {
        if state.step == .cloudCheck, step == .restoreOffer {
            cloudCheckWarning = state.errorMessage
        } else if step != .restoreOffer {
            cloudCheckWarning = nil
        }

        state.step = step
    }
}

struct OnboardingContainer: View {
    @State private var manager: OnboardingManager
    let onComplete: () -> Void

    init(manager: OnboardingManager, onComplete: @escaping () -> Void) {
        _manager = State(initialValue: manager)
        self.onComplete = onComplete
    }

    var body: some View {
        stepView(for: manager.state.step)
            .onChange(of: manager.isComplete) { _, complete in
                guard complete else { return }
                manager.app.reloadWallets()
                onComplete()
            }
    }

    @ViewBuilder
    private func stepView(for step: OnboardingStep) -> some View {
        switch step {
        case .terms:
            TermsAndConditionsView {
                manager.dispatch(.acceptTerms)
            }

        case .cloudCheck:
            CloudCheckContent()

        case .restoreOffer:
            CloudRestoreOfferView(
                onRestore: {
                    manager.cloudCheckWarning = nil
                    manager.dispatch(.startRestore)
                },
                onSkip: {
                    manager.cloudCheckWarning = nil
                    manager.dispatch(.skipRestore)
                },
                warningMessage: manager.cloudCheckWarning,
                errorMessage: manager.cloudCheckWarning == nil ? manager.state.errorMessage : nil
            )

        case .restoring:
            DeviceRestoreView(
                onComplete: { manager.dispatch(.restoreComplete) },
                onError: { error in manager.dispatch(.restoreFailed(error: error)) }
            )

        case .welcome:
            OnboardingWelcomeScreen {
                manager.dispatch(.continueFromWelcome)
            }

        case .bitcoinChoice:
            OnboardingBitcoinChoiceScreen(
                onNewHere: { manager.dispatch(.selectHasBitcoin(hasBitcoin: false)) },
                onHasBitcoin: { manager.dispatch(.selectHasBitcoin(hasBitcoin: true)) }
            )

        case .storageChoice:
            OnboardingStorageChoiceScreen(
                onExchange: {
                    manager.dispatch(.selectStorage(selection: .exchange))
                },
                onHardwareWallet: {
                    manager.dispatch(.selectStorage(selection: .hardwareWallet))
                },
                onSoftwareWallet: {
                    manager.dispatch(.selectStorage(selection: .softwareWallet))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .softwareChoice:
            OnboardingSoftwareChoiceScreen(
                onCreateWallet: {
                    manager.dispatch(.selectSoftwareAction(selection: .createNewWallet))
                },
                onImportWallet: {
                    manager.dispatch(.selectSoftwareAction(selection: .importExistingWallet))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .creatingWallet:
            OnboardingCreatingWalletView {
                manager.dispatch(.continueWalletCreation)
            }

        case .backupWallet:
            OnboardingBackupWalletView(
                branch: manager.state.branch,
                secretWordsSaved: manager.state.secretWordsSaved,
                cloudBackupEnabled: manager.state.cloudBackupEnabled,
                wordCount: manager.state.createdWords.count,
                onShowWords: { manager.dispatch(.showSecretWords) },
                onEnableCloudBackup: { manager.dispatch(.openCloudBackup) },
                onContinue: { manager.dispatch(.continueFromBackup) }
            )

        case .cloudBackup:
            OnboardingCloudBackupStepView(
                onEnabled: { manager.dispatch(.cloudBackupEnabled) },
                onSkip: { manager.dispatch(.skipCloudBackup) }
            )

        case .secretWords:
            OnboardingSecretWordsView(
                words: manager.state.createdWords,
                onBack: { manager.dispatch(.back) },
                onSaved: { manager.dispatch(.secretWordsSaved) }
            )

        case .verifyWords:
            if let walletId = manager.rust.currentWalletId() {
                VerifyWordsContainer(
                    id: walletId,
                    onVerified: { manager.dispatch(.verifyWordsCompleted) }
                )
            } else {
                OnboardingErrorScreen(
                    title: "Unable to verify words",
                    message: "The wallet was created, but the verification state could not be loaded."
                )
            }

        case .exchangeFunding:
            OnboardingExchangeFundingView(
                walletId: manager.rust.currentWalletId(),
                onContinue: { manager.dispatch(.continueFromExchangeFunding) }
            )

        case .hardwareDeviceSelection:
            OnboardingHardwareDeviceSelectionScreen(
                selectedDevice: manager.state.hardwareDevice,
                onSelect: { device in
                    manager.dispatch(.selectHardwareDevice(device: device))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .hardwareImport:
            OnboardingHardwareImportFlowView(
                device: manager.state.hardwareDevice,
                onImported: { walletId in
                    manager.dispatch(.hardwareImportCompleted(walletId: walletId))
                },
                onBack: { manager.dispatch(.back) }
            )

        case .softwareImport:
            OnboardingSoftwareImportFlowView(
                onImported: { walletId in
                    manager.dispatch(.softwareImportCompleted(walletId: walletId))
                },
                onBackupImported: {
                    manager.dispatch(.backupImportCompleted)
                },
                onBack: { manager.dispatch(.back) }
            )
        }
    }
}

private struct CloudCheckContent: View {
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

private struct OnboardingWelcomeScreen: View {
    let onContinue: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "sparkles",
            title: "Welcome to Cove",
            subtitle: "A self-custody Bitcoin wallet focused on secure backups, clear flows, and hardware wallet support."
        ) {
            Button("Get Started", action: onContinue)
                .buttonStyle(OnboardingPrimaryButtonStyle())
        }
    }
}

private struct OnboardingBitcoinChoiceScreen: View {
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

private struct OnboardingStorageChoiceScreen: View {
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

private struct OnboardingSoftwareChoiceScreen: View {
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

private struct OnboardingCreatingWalletView: View {
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
            didAdvance = true
            try? await Task.sleep(for: .milliseconds(900))
            onContinue()
        }
    }
}

private struct OnboardingBackupWalletView: View {
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

private struct OnboardingSecretWordsView: View {
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

                    Button("I Saved These Words", action: onSaved)
                        .buttonStyle(OnboardingPrimaryButtonStyle())
                }
                .padding(.horizontal, 24)
                .padding(.bottom, 28)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onboardingRecoveryBackground()
    }
}

private struct OnboardingCloudBackupStepView: View {
    @State private var backupManager = CloudBackupManager.shared
    @State private var didComplete = false

    let onEnabled: () -> Void
    let onSkip: () -> Void

    var body: some View {
        ZStack {
            CloudBackupEnableOnboardingView(
                onEnable: {
                    backupManager.dispatch(action: .enableCloudBackupNoDiscovery)
                },
                onCancel: onSkip
            )

            if case .enabling = backupManager.status {
                Color.black.opacity(0.35)
                    .ignoresSafeArea()

                VStack(spacing: 12) {
                    ProgressView()
                        .tint(.white)
                    Text("Enabling Cloud Backup")
                        .font(.headline)
                        .foregroundStyle(.white)
                }
            }
        }
        .onChange(of: backupManager.status) { _, status in
            guard !didComplete else { return }
            guard case .enabled = status else { return }
            didComplete = true
            onEnabled()
        }
    }
}

private struct OnboardingExchangeFundingView: View {
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

private struct OnboardingHardwareDeviceSelectionScreen: View {
    let selectedDevice: OnboardingHardwareDevice?
    let onSelect: (OnboardingHardwareDevice) -> Void
    let onBack: () -> Void

    var body: some View {
        OnboardingPromptScreen(
            icon: "shield.lefthalf.filled",
            title: "Which hardware wallet do you use?",
            subtitle: "Import the wallet you already have without moving your keys onto this device."
        ) {
            VStack(spacing: 14) {
                ForEach(OnboardingHardwareDevice.allCases, id: \.self) { device in
                    OnboardingChoiceCard(
                        title: device.title,
                        subtitle: device.subtitle,
                        systemImage: device.systemImage,
                        isSelected: selectedDevice == device
                    ) {
                        onSelect(device)
                    }
                }
            }

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }
}

private struct OnboardingSoftwareImportFlowView: View {
    enum Mode {
        case chooser
        case wordCount
        case words(NumberOfBip39Words)
        case qr
        case backupFile
    }

    @State private var mode: Mode = .chooser

    let onImported: (WalletId) -> Void
    let onBackupImported: () -> Void
    let onBack: () -> Void

    var body: some View {
        switch mode {
        case .chooser:
            OnboardingPromptScreen(
                icon: "square.and.arrow.down.on.square",
                title: "Import your software wallet",
                subtitle: "Choose how you want to bring your existing wallet into Cove."
            ) {
                VStack(spacing: 14) {
                    OnboardingChoiceCard(
                        title: "Enter recovery words",
                        subtitle: "Import a 12- or 24-word recovery phrase",
                        systemImage: "keyboard"
                    ) {
                        mode = .wordCount
                    }

                    OnboardingChoiceCard(
                        title: "Scan QR code",
                        subtitle: "Scan a mnemonic QR from another wallet",
                        systemImage: "qrcode.viewfinder"
                    ) {
                        mode = .qr
                    }

                    OnboardingChoiceCard(
                        title: "Import Cove backup file",
                        subtitle: "Restore from a previously exported encrypted backup",
                        systemImage: "doc.badge.plus"
                    ) {
                        mode = .backupFile
                    }
                }

                Button("Back", action: onBack)
                    .buttonStyle(OnboardingSecondaryButtonStyle())
            }

        case .wordCount:
            OnboardingPromptScreen(
                icon: "list.number",
                title: "How many words do you have?",
                subtitle: "Select the recovery phrase length before entering your words."
            ) {
                VStack(spacing: 14) {
                    OnboardingChoiceCard(
                        title: "12 words",
                        subtitle: "Most modern wallet backups",
                        systemImage: "12.circle"
                    ) {
                        mode = .words(.twelve)
                    }

                    OnboardingChoiceCard(
                        title: "24 words",
                        subtitle: "Some wallets use a longer phrase",
                        systemImage: "24.circle"
                    ) {
                        mode = .words(.twentyFour)
                    }
                }

                Button("Back") { mode = .chooser }
                    .buttonStyle(OnboardingSecondaryButtonStyle())
            }

        case let .words(numberOfWords):
            OnboardingEmbeddedNavigation(title: "Import Recovery Words", onBack: {
                mode = .wordCount
            }) {
                HotWalletImportScreen(numberOfWords: numberOfWords, onImported: onImported)
            }

        case .qr:
            OnboardingEmbeddedNavigation(title: "Scan Recovery QR", onBack: {
                mode = .chooser
            }) {
                HotWalletImportScreen(
                    numberOfWords: .twelve,
                    importType: .qr,
                    onImported: onImported
                )
            }

        case .backupFile:
            OnboardingEmbeddedNavigation(title: "Import Backup File", onBack: {
                mode = .chooser
            }) {
                BackupImportView(onImported: onBackupImported)
            }
        }
    }
}

private struct OnboardingHardwareImportFlowView: View {
    enum Mode {
        case chooser
        case qr
        case file
        case nfc
    }

    let device: OnboardingHardwareDevice?
    let onImported: (WalletId) -> Void
    let onBack: () -> Void

    @State private var mode: Mode = .chooser

    private var supportsNfc: Bool {
        device == .coldcard
    }

    var body: some View {
        switch mode {
        case .chooser:
            OnboardingPromptScreen(
                icon: "arrow.down.doc",
                title: "Import your hardware wallet",
                subtitle: "Choose an export method supported by your device."
            ) {
                VStack(spacing: 14) {
                    OnboardingChoiceCard(
                        title: "Scan export QR",
                        subtitle: "Use the QR export from your hardware wallet",
                        systemImage: "qrcode.viewfinder"
                    ) {
                        mode = .qr
                    }

                    OnboardingChoiceCard(
                        title: "Import export file",
                        subtitle: "Use a wallet export file from your device",
                        systemImage: "doc"
                    ) {
                        mode = .file
                    }

                    if supportsNfc {
                        OnboardingChoiceCard(
                            title: "Scan with NFC",
                            subtitle: "Tap your device to this iPhone",
                            systemImage: "wave.3.right"
                        ) {
                            mode = .nfc
                        }
                    }
                }

                Button("Back", action: onBack)
                    .buttonStyle(OnboardingSecondaryButtonStyle())
            }

        case .qr:
            OnboardingEmbeddedNavigation(title: "Scan Hardware QR", onBack: {
                mode = .chooser
            }) {
                QrCodeImportScreen(onImported: onImported)
            }

        case .file:
            OnboardingHardwareFileImportView(
                onImported: onImported,
                onBack: { mode = .chooser }
            )

        case .nfc:
            OnboardingHardwareNfcImportView(
                onImported: onImported,
                onBack: { mode = .chooser }
            )
        }
    }
}

private struct OnboardingHardwareFileImportView: View {
    let onImported: (WalletId) -> Void
    let onBack: () -> Void

    @State private var showingFilePicker = false
    @State private var errorMessage: String?
    @State private var isImporting = false

    var body: some View {
        OnboardingPromptScreen(
            icon: "doc.text",
            title: "Import a hardware export file",
            subtitle: "Choose the wallet export file from your hardware wallet."
        ) {
            if let errorMessage {
                OnboardingInlineMessage(text: errorMessage)
            }

            Button {
                showingFilePicker = true
            } label: {
                if isImporting {
                    HStack {
                        Spacer()
                        ProgressView()
                            .tint(.white)
                        Spacer()
                    }
                } else {
                    Text("Choose File")
                }
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())
            .disabled(isImporting)

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
        .fileImporter(
            isPresented: $showingFilePicker,
            allowedContentTypes: [.plainText, .json, .data]
        ) { result in
            switch result {
            case let .success(url):
                importFile(url)
            case let .failure(error):
                errorMessage = error.localizedDescription
            }
        }
    }

    private func importFile(_ url: URL) {
        isImporting = true
        defer { isImporting = false }

        do {
            let multiFormat = try FileHandler(filePath: url.absoluteString).read()
            guard case let .hardwareExport(export) = multiFormat else {
                errorMessage = "That file doesn’t contain a hardware wallet export."
                return
            }

            let wallet = try Wallet.newFromExport(export: export)
            onImported(wallet.id())
        } catch let WalletError.WalletAlreadyExists(walletId) {
            onImported(walletId)
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

private struct OnboardingHardwareNfcImportView: View {
    let onImported: (WalletId) -> Void
    let onBack: () -> Void

    @State private var reader = NFCReader()
    @State private var errorMessage: String?

    var body: some View {
        OnboardingPromptScreen(
            icon: "wave.3.right",
            title: "Scan your hardware wallet with NFC",
            subtitle: "Hold your hardware wallet or export tag near the top of your iPhone."
        ) {
            if let errorMessage {
                OnboardingInlineMessage(text: errorMessage)
            }

            Button("Start NFC Scan") {
                errorMessage = nil
                reader.scan()
            }
            .buttonStyle(OnboardingPrimaryButtonStyle())

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
        .onChange(of: reader.scannedMessage) { _, message in
            guard let message else { return }
            handleNfcMessage(message)
        }
        .onDisappear {
            reader.resetReader()
            reader.session = nil
        }
    }

    private func handleNfcMessage(_ message: NfcMessage) {
        do {
            let multiFormat = try message.tryIntoMultiFormat()
            guard case let .hardwareExport(export) = multiFormat else {
                errorMessage = "That NFC payload doesn’t contain a hardware wallet export."
                return
            }

            let wallet = try Wallet.newFromExport(export: export)
            onImported(wallet.id())
        } catch let WalletError.WalletAlreadyExists(walletId) {
            onImported(walletId)
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

private struct OnboardingEmbeddedNavigation<Content: View>: View {
    let title: String
    let onBack: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(title)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Back", action: onBack)
                    }
                }
        }
    }
}

private struct OnboardingPromptScreen<Footer: View>: View {
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

private struct OnboardingChoiceCard: View {
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

private struct OnboardingStatusCard: View {
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

private struct OnboardingInlineMessage: View {
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

private struct OnboardingWordCard: View {
    let index: Int
    let word: String

    var body: some View {
        HStack(spacing: 10) {
            Text("\(index)")
                .font(.caption.weight(.semibold))
                .foregroundStyle(Color.btnGradientLight)
                .frame(width: 24)

            Text(word)
                .font(.system(.body, design: .monospaced).weight(.medium))
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

private struct OnboardingAddressQr: View {
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

private struct OnboardingErrorScreen: View {
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

private extension OnboardingHardwareDevice {
    static let allCases: [OnboardingHardwareDevice] = [.coldcard, .ledger, .trezor, .other]

    var title: String {
        switch self {
        case .coldcard:
            "Coldcard"
        case .ledger:
            "Ledger"
        case .trezor:
            "Trezor"
        case .other:
            "Other hardware wallet"
        }
    }

    var subtitle: String {
        switch self {
        case .coldcard:
            "Import via QR, file, or NFC"
        case .ledger:
            "Import via QR or file"
        case .trezor:
            "Import via QR or file"
        case .other:
            "Import via QR or file"
        }
    }

    var systemImage: String {
        switch self {
        case .coldcard:
            "creditcard.and.123"
        case .ledger:
            "lanyardcard"
        case .trezor:
            "shield.lefthalf.filled"
        case .other:
            "externaldrive"
        }
    }
}

#Preview("Cloud Check") {
    CloudCheckContent()
}
