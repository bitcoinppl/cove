import SwiftUI
import UniformTypeIdentifiers

struct OnboardingHardwareDeviceSelectionScreen: View {
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

struct OnboardingSoftwareImportFlowView: View {
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

struct OnboardingHardwareImportFlowView: View {
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

struct OnboardingHardwareFileImportView: View {
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
        errorMessage = nil
        isImporting = true
        defer { isImporting = false }

        let didAccess = url.startAccessingSecurityScopedResource()
        defer {
            if didAccess {
                url.stopAccessingSecurityScopedResource()
            }
        }

        do {
            let multiFormat = try FileHandler(filePath: url.path).read()
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

struct OnboardingHardwareNfcImportView: View {
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

struct OnboardingEmbeddedNavigation<Content: View>: View {
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

extension OnboardingHardwareDevice {
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
