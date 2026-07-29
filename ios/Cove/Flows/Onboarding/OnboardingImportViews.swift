import SwiftUI

struct OnboardingSoftwareImportFlowView: View {
    enum Mode {
        case chooser
        case wordCount
        case words(NumberOfBip39Words)
        case qr
    }

    @State private var mode: Mode = .chooser

    let errorMessage: String?
    let cloudRestoreAlertVisible: Binding<Bool>
    let onImported: (WalletId) -> Void
    let onCreateWallet: () -> Void
    let onRestoreFromCloudBackup: () -> Void
    let onDismissCloudRestoreAlert: () -> Void
    let onBack: () -> Void

    var body: some View {
        OnboardingSoftwareImportContent(
            errorMessage: errorMessage,
            onImported: onImported,
            onCreateWallet: onCreateWallet,
            onBack: onBack,
            mode: $mode
        )
        .cloudRestoreAlert(
            isPresented: cloudRestoreAlertVisible,
            onRestore: onRestoreFromCloudBackup,
            onContinue: onDismissCloudRestoreAlert
        )
    }
}

private struct OnboardingSoftwareImportContent: View {
    let errorMessage: String?
    let onImported: (WalletId) -> Void
    let onCreateWallet: () -> Void
    let onBack: () -> Void

    @Binding var mode: OnboardingSoftwareImportFlowView.Mode

    var body: some View {
        switch mode {
        case .chooser:
            OnboardingSoftwareImportChooser(
                errorMessage: errorMessage,
                onCreateWallet: onCreateWallet,
                onBack: onBack,
                mode: $mode
            )

        case .wordCount:
            OnboardingSoftwareImportWordCount(mode: $mode)

        case let .words(numberOfWords):
            OnboardingSoftwareWordsImport(
                numberOfWords: numberOfWords,
                onImported: onImported,
                mode: $mode
            )

        case .qr:
            OnboardingSoftwareQrImport(onImported: onImported, mode: $mode)
        }
    }
}

private struct OnboardingSoftwareImportChooser: View {
    let errorMessage: String?
    let onCreateWallet: () -> Void
    let onBack: () -> Void

    @Binding var mode: OnboardingSoftwareImportFlowView.Mode

    var body: some View {
        OnboardingPromptScreen(
            icon: "square.and.arrow.down.on.square",
            title: "Import your software wallet",
            subtitle: "Choose how you want to bring your existing wallet into Cove."
        ) {
            if let errorMessage {
                OnboardingInlineMessage(text: errorMessage)
            }

            VStack(spacing: 14) {
                OnboardingChoiceCard(
                    title: "Enter recovery words",
                    subtitle: "Import a 12- or 24-word recovery phrase",
                    systemImage: "keyboard",
                    action: chooseWords
                )

                OnboardingChoiceCard(
                    title: "Scan QR code",
                    subtitle: "Scan a mnemonic QR from another wallet",
                    systemImage: "qrcode.viewfinder",
                    action: chooseQr
                )
            }

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())

            Button("Create a new wallet instead", action: onCreateWallet)
                .font(OnboardingRecoveryTypography.bodySemibold)
                .foregroundStyle(.white.opacity(0.68))
                .frame(maxWidth: .infinity)
                .padding(.top, 2)
                .contentShape(Rectangle())
                .buttonStyle(.plain)
        }
    }

    private func chooseWords() {
        mode = .wordCount
    }

    private func chooseQr() {
        mode = .qr
    }
}

private struct OnboardingSoftwareImportWordCount: View {
    @Binding var mode: OnboardingSoftwareImportFlowView.Mode

    var body: some View {
        OnboardingPromptScreen(
            icon: "list.number",
            title: "How many words do you have?",
            subtitle: "Select the recovery phrase length before entering your words."
        ) {
            VStack(spacing: 14) {
                OnboardingChoiceCard(
                    title: "12 words",
                    subtitle: "Most modern wallet backups",
                    systemImage: "12.circle",
                    action: chooseTwelve
                )

                OnboardingChoiceCard(
                    title: "24 words",
                    subtitle: "Some wallets use a longer phrase",
                    systemImage: "24.circle",
                    action: chooseTwentyFour
                )
            }

            Button("Back", action: goBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }

    private func chooseTwelve() {
        mode = .words(.twelve)
    }

    private func chooseTwentyFour() {
        mode = .words(.twentyFour)
    }

    private func goBack() {
        mode = .chooser
    }
}

private struct OnboardingSoftwareWordsImport: View {
    let numberOfWords: NumberOfBip39Words
    let onImported: (WalletId) -> Void

    @Binding var mode: OnboardingSoftwareImportFlowView.Mode

    var body: some View {
        OnboardingEmbeddedNavigation(
            title: "Import Recovery Words",
            onBack: { mode = .wordCount }
        ) {
            HotWalletImportScreen(numberOfWords: numberOfWords, onImported: onImported)
        }
    }
}

private struct OnboardingSoftwareQrImport: View {
    let onImported: (WalletId) -> Void

    @Binding var mode: OnboardingSoftwareImportFlowView.Mode

    var body: some View {
        OnboardingEmbeddedNavigation(
            title: "Scan Recovery QR",
            onBack: { mode = .chooser }
        ) {
            HotWalletImportScreen(
                numberOfWords: .twelve,
                importType: .qr,
                autoImportScannedWords: true,
                onImported: onImported
            )
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

    @State private var mode: Mode = .chooser

    let cloudRestoreAlertVisible: Binding<Bool>
    let onImported: (WalletId) -> Void
    let onRestoreFromCloudBackup: () -> Void
    let onDismissCloudRestoreAlert: () -> Void
    let onBack: () -> Void

    var body: some View {
        OnboardingHardwareImportContent(
            onImported: onImported,
            onBack: onBack,
            mode: $mode
        )
        .cloudRestoreAlert(
            isPresented: cloudRestoreAlertVisible,
            onRestore: onRestoreFromCloudBackup,
            onContinue: onDismissCloudRestoreAlert
        )
    }
}

private struct OnboardingHardwareImportContent: View {
    let onImported: (WalletId) -> Void
    let onBack: () -> Void

    @Binding var mode: OnboardingHardwareImportFlowView.Mode

    var body: some View {
        switch mode {
        case .chooser:
            OnboardingHardwareImportChooser(
                onBack: onBack,
                mode: $mode
            )

        case .qr:
            OnboardingHardwareQrImport(onImported: onImported, mode: $mode)

        case .file:
            OnboardingHardwareFileImportStep(onImported: onImported, mode: $mode)

        case .nfc:
            OnboardingHardwareNfcImportStep(onImported: onImported, mode: $mode)
        }
    }
}

private struct OnboardingHardwareImportChooser: View {
    let onBack: () -> Void

    @Binding var mode: OnboardingHardwareImportFlowView.Mode

    var body: some View {
        OnboardingPromptScreen(
            icon: "arrow.down.doc",
            title: "Import your hardware wallet",
            subtitle: "Choose how your hardware wallet exports its public data."
        ) {
            VStack(spacing: 14) {
                OnboardingChoiceCard(
                    title: "Scan export QR",
                    subtitle: "Use the QR export from your hardware wallet",
                    systemImage: "qrcode.viewfinder",
                    action: chooseQr
                )

                OnboardingChoiceCard(
                    title: "Import export file",
                    subtitle: "Use a wallet export file from your device",
                    systemImage: "doc",
                    action: chooseFile
                )

                OnboardingChoiceCard(
                    title: "Scan with NFC",
                    subtitle: "Hold your hardware wallet or export tag near the top of your iPhone.",
                    systemImage: "wave.3.right",
                    action: chooseNfc
                )
            }

            Button("Back", action: onBack)
                .buttonStyle(OnboardingSecondaryButtonStyle())
        }
    }

    private func chooseQr() {
        mode = .qr
    }

    private func chooseFile() {
        mode = .file
    }

    private func chooseNfc() {
        mode = .nfc
    }
}

private struct OnboardingHardwareQrImport: View {
    let onImported: (WalletId) -> Void

    @Binding var mode: OnboardingHardwareImportFlowView.Mode

    var body: some View {
        OnboardingEmbeddedNavigation(
            title: "Scan Hardware QR",
            onBack: { mode = .chooser }
        ) {
            QrCodeImportScreen(onImported: onImported)
        }
    }
}

private struct OnboardingHardwareFileImportStep: View {
    let onImported: (WalletId) -> Void

    @Binding var mode: OnboardingHardwareImportFlowView.Mode

    var body: some View {
        OnboardingHardwareFileImportView(
            onImported: onImported,
            onBack: { mode = .chooser }
        )
    }
}

private struct OnboardingHardwareNfcImportStep: View {
    let onImported: (WalletId) -> Void

    @Binding var mode: OnboardingHardwareImportFlowView.Mode

    var body: some View {
        OnboardingHardwareNfcImportView(
            onImported: onImported,
            onBack: { mode = .chooser }
        )
    }
}

private extension View {
    func cloudRestoreAlert(
        isPresented: Binding<Bool>,
        onRestore: @escaping () -> Void,
        onContinue: @escaping () -> Void
    ) -> some View {
        alert("Cove backup found", isPresented: isPresented) {
            Button("Restore from Cove backup", action: onRestore)
            Button("Continue setup", role: .cancel, action: onContinue)
        } message: {
            Text("We found a cloud backup for this account.")
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
