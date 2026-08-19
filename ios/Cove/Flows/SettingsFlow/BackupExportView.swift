import AuthenticationServices
import Security
import SwiftUI

private enum BackupExportPresentation {
    case passwordSetupOptions
    case exportConfirmation
}

struct BackupExportView: View {
    @Environment(\.dismiss) private var dismiss
    @FocusState private var isManualPasswordFieldFocused: Bool

    @State private var password = ""
    @State private var generatedPassword: String? = nil
    @State private var isPasswordVisible = false
    @State private var isManualPasswordEntryVisible = false
    @State private var isExporting = false
    @State private var errorMessage: String? = nil
    @State private var warningMessage: String? = nil
    @State private var exportTask: Task<Void, Never>? = nil
    @State private var tempFileURL: URL? = nil
    @State private var shareSheetPresented = false
    @State private var showSaveToPasswords = false
    @State private var pendingExportAfterPasswordSetup = false
    @State private var passwordCopiedMessage: String? = nil
    @State private var passwordDelegate: PasswordRetrievalDelegate? = nil

    @State private var backupManager = BackupManager()
    @State private var presentationCoordinator =
        PresentationTransitionCoordinator<BackupExportPresentation>()

    private var isPasswordValid: Bool {
        backupManager.isPasswordValid(password: password)
    }

    private var passwordStatusText: String {
        if isManualPasswordEntryVisible {
            return "Manual backup password ready"
        }

        return "Backup password ready"
    }

    private var exportConfirmationIsPresented: Binding<Bool> {
        presentationCoordinator.isPresented { presentation in
            if case .exportConfirmation = presentation { true } else { false }
        }
    }

    private var passwordSetupOptionsIsPresented: Binding<Bool> {
        presentationCoordinator.isPresented { presentation in
            if case .passwordSetupOptions = presentation { true } else { false }
        }
    }

    var body: some View {
        PasswordCopiedAlert(
            message: $passwordCopiedMessage,
            continueExport: continuePendingExportIfReady,
            content: SaveToPasswordsAlert(
                isPresented: $showSaveToPasswords,
                save: saveToPasswords,
                skip: continuePendingExportIfReady,
                content: ExportWarningAlert(
                    message: $warningMessage,
                    content: ExportFailureAlert(
                        message: $errorMessage,
                        content: ExportConfirmationAlert(
                            isPresented: exportConfirmationIsPresented,
                            export: exportBackup,
                            content: BackupExportForm(
                                password: $password,
                                isPasswordVisible: $isPasswordVisible,
                                isManualPasswordEntryVisible: isManualPasswordEntryVisible,
                                isManualPasswordFieldFocused: $isManualPasswordFieldFocused,
                                generatedPassword: generatedPassword,
                                isPasswordValid: isPasswordValid,
                                passwordStatusText: passwordStatusText,
                                isExporting: isExporting,
                                actions: BackupPasswordActions(
                                    generate: prepareGeneratedPassword,
                                    retrieve: retrieveFromPasswords,
                                    toggleManualEntry: showManualPasswordEntry,
                                    copy: copyPassword
                                ),
                                passwordSetupOptions: BackupPasswordSetupOptions(
                                    isPresented: passwordSetupOptionsIsPresented,
                                    generate: prepareGeneratedPassword,
                                    retrieve: retrieveFromPasswords,
                                    enterManually: showManualPasswordEntry,
                                    cancel: cancelPendingExport
                                ),
                                saveToPasswords: saveToPasswords,
                                clearGeneratedPassword: clearGeneratedPassword,
                                export: handleExportTapped
                            )
                            .onDisappear(perform: handleDisappear)
                        )
                    )
                )
            )
        )
        .presentationTransitionHost(presentationCoordinator)
    }

    private func prepareGeneratedPassword() {
        dismissPasswordEntryUI()
        let generated = backupManager.generatePassword()
        generatedPassword = generated
        password = generated
    }

    private func clearGeneratedPassword() {
        generatedPassword = nil
        password = ""
    }

    private func copyPassword() {
        UIPasteboard.general.setItems(
            [["public.utf8-plain-text": password]],
            options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(120)]
        )
    }

    private func showManualPasswordEntry() {
        isManualPasswordEntryVisible.toggle()

        guard isManualPasswordEntryVisible else {
            dismissPasswordEntryUI()
            return
        }

        Task { @MainActor in
            await Task.yield()
            isManualPasswordFieldFocused = true
        }
    }

    private func dismissPasswordEntryUI() {
        isManualPasswordFieldFocused = false
        isManualPasswordEntryVisible = false
    }

    private func handleExportTapped() {
        dismissPasswordEntryUI()

        guard isPasswordValid else {
            pendingExportAfterPasswordSetup = true

            if password.isEmpty {
                presentationCoordinator.present(.passwordSetupOptions)
            } else {
                showManualPasswordEntry()
            }
            return
        }

        presentationCoordinator.present(.exportConfirmation)
    }

    private func continuePendingExportIfReady() {
        guard pendingExportAfterPasswordSetup, isPasswordValid else { return }

        pendingExportAfterPasswordSetup = false
        presentationCoordinator.transitionAfterPresenterDismissal(
            to: .exportConfirmation
        )
    }

    private func cancelPendingExport() {
        pendingExportAfterPasswordSetup = false
    }

    private func handleDisappear() {
        exportTask?.cancel()
        password = ""

        guard let url = tempFileURL else { return }

        do {
            try FileManager.default.removeItem(at: url)
        } catch {
            Log.warn("Failed to delete temp backup file: \(error.localizedDescription)")
        }
        tempFileURL = nil
    }

    private func saveToPasswords() {
        let account = backupManager.backupAccountName() as CFString

        SecAddSharedWebCredential(
            "covebitcoinwallet.com" as CFString,
            account,
            password as CFString
        ) { error in
            DispatchQueue.main.async {
                guard error != nil else {
                    continuePendingExportIfReady()
                    return
                }

                UIPasteboard.general.setItems(
                    [["public.utf8-plain-text": password]],
                    options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(120)]
                )
                passwordCopiedMessage = "Password copied to clipboard. Save it in your password manager (1Password, etc.) under covebitcoinwallet.com so you can retrieve it later."
            }
        }
    }

    private func retrieveFromPasswords() {
        dismissPasswordEntryUI()
        let provider = ASAuthorizationPasswordProvider()
        let request = provider.createRequest()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        let delegate = PasswordRetrievalDelegate(
            onPassword: { retrievedPassword in
                password = retrievedPassword
                continuePendingExportIfReady()
            },
            onError: { msg in
                pendingExportAfterPasswordSetup = false
                errorMessage = msg
            }
        )
        passwordDelegate = delegate
        controller.delegate = delegate
        controller.performRequests()
    }

    private func exportBackup() {
        isExporting = true
        exportTask = Task {
            var localFileURL: URL?
            do {
                let result = try await backupManager.export(password: password)

                let fileURL = FileManager.default.temporaryDirectory.appendingPathComponent(result.filename)
                try result.data.write(to: fileURL, options: [.atomic, .completeFileProtection])
                localFileURL = fileURL

                let warnings = result.warnings

                try Task.checkCancellation()

                await MainActor.run {
                    tempFileURL = fileURL
                    isExporting = false

                    let exportWarning = warnings.isEmpty
                        ? nil
                        : "Some data could not be exported:\n" + warnings.joined(separator: "\n")

                    shareSheetPresented = true
                    ShareSheet.present(for: fileURL) { completed in
                        shareSheetPresented = false
                        do {
                            try FileManager.default.removeItem(at: fileURL)
                        } catch {
                            Log.warn("Failed to delete temp backup file: \(error.localizedDescription)")
                        }
                        tempFileURL = nil

                        guard completed else { return }
                        if let exportWarning {
                            warningMessage = exportWarning
                        } else {
                            dismiss()
                        }
                    }
                }
            } catch {
                for url in [tempFileURL, localFileURL].compactMap(\.self) {
                    try? FileManager.default.removeItem(at: url)
                }
                await MainActor.run {
                    tempFileURL = nil
                    isExporting = false
                    errorMessage = (error as? BackupError)?.description ?? error.localizedDescription
                }
            }
        }
    }
}

private struct BackupExportForm: View {
    @Binding var password: String
    @Binding var isPasswordVisible: Bool

    let isManualPasswordEntryVisible: Bool
    let isManualPasswordFieldFocused: FocusState<Bool>.Binding
    let generatedPassword: String?
    let isPasswordValid: Bool
    let passwordStatusText: String
    let isExporting: Bool
    let actions: BackupPasswordActions
    let passwordSetupOptions: BackupPasswordSetupOptions
    let saveToPasswords: () -> Void
    let clearGeneratedPassword: () -> Void
    let export: () -> Void

    var body: some View {
        Form {
            if let generated = generatedPassword {
                GeneratedBackupPasswordSection(
                    password: generated,
                    saveToPasswords: saveToPasswords,
                    clearPassword: clearGeneratedPassword
                )
            }

            BackupExportPasswordSection(
                password: $password,
                isPasswordVisible: $isPasswordVisible,
                isManualPasswordEntryVisible: isManualPasswordEntryVisible,
                isManualPasswordFieldFocused: isManualPasswordFieldFocused,
                generatedPassword: generatedPassword,
                isPasswordValid: isPasswordValid,
                passwordStatusText: passwordStatusText,
                actions: actions
            )

            BackupSecurityWarningSection()

            BackupExportActionSection(
                isExporting: isExporting,
                passwordSetupOptions: passwordSetupOptions,
                export: export
            )
        }
    }
}

private struct GeneratedBackupPasswordSection: View {
    let password: String
    let saveToPasswords: () -> Void
    let clearPassword: () -> Void

    var body: some View {
        Section {
            Text(password)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)

            Button(action: copyPassword) {
                Label("Copy Password", systemImage: "doc.on.doc")
            }

            Button(action: saveToPasswords) {
                Label("Save to Apple Passwords", systemImage: "lock.shield")
            }

            Button(role: .destructive, action: clearPassword) {
                Label("Clear", systemImage: "xmark.circle")
            }
        } header: {
            Text("Generated Backup Password")
        } footer: {
            Text("Using a third-party password manager? Copy the password and save it manually")
        }
    }

    private func copyPassword() {
        UIPasteboard.general.setItems(
            [["public.utf8-plain-text": password]],
            options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(120)]
        )
    }
}

private struct BackupPasswordActions {
    let generate: () -> Void
    let retrieve: () -> Void
    let toggleManualEntry: () -> Void
    let copy: () -> Void
}

private struct BackupExportPasswordSection: View {
    @Binding var password: String
    @Binding var isPasswordVisible: Bool

    let isManualPasswordEntryVisible: Bool
    let isManualPasswordFieldFocused: FocusState<Bool>.Binding
    let generatedPassword: String?
    let isPasswordValid: Bool
    let passwordStatusText: String
    let actions: BackupPasswordActions

    var body: some View {
        Section {
            BackupPasswordSectionContent(
                password: $password,
                isPasswordVisible: $isPasswordVisible,
                isManualPasswordEntryVisible: isManualPasswordEntryVisible,
                isManualPasswordFieldFocused: isManualPasswordFieldFocused,
                generatedPassword: generatedPassword,
                passwordStatusText: passwordStatusText,
                actions: actions
            )
        } header: {
            BackupPasswordSectionHeader(hasGeneratedPassword: generatedPassword != nil)
        } footer: {
            BackupPasswordSectionFooter(
                password: password,
                isPasswordValid: isPasswordValid,
                hasGeneratedPassword: generatedPassword != nil
            )
        }
    }
}

private struct BackupPasswordSectionContent: View {
    @Binding var password: String
    @Binding var isPasswordVisible: Bool

    let isManualPasswordEntryVisible: Bool
    let isManualPasswordFieldFocused: FocusState<Bool>.Binding
    let generatedPassword: String?
    let passwordStatusText: String
    let actions: BackupPasswordActions

    var body: some View {
        if isManualPasswordEntryVisible {
            ManualBackupPasswordField(
                password: $password,
                isPasswordVisible: $isPasswordVisible,
                isFocused: isManualPasswordFieldFocused
            )
        }

        if generatedPassword == nil, !password.isEmpty {
            Label(passwordStatusText, systemImage: "checkmark.circle.fill")
                .foregroundColor(.secondary)
        }

        BackupPasswordActionButtons(
            canGenerate: generatedPassword == nil,
            canCopy: generatedPassword == nil && !password.isEmpty,
            isManualPasswordEntryVisible: isManualPasswordEntryVisible,
            actions: actions
        )
    }
}

private struct ManualBackupPasswordField: View {
    @Binding var password: String
    @Binding var isPasswordVisible: Bool

    let isFocused: FocusState<Bool>.Binding

    var body: some View {
        HStack {
            if isPasswordVisible {
                TextField("Password", text: $password)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                    .textContentType(.newPassword)
                    .focused(isFocused)
            } else {
                SecureField("Password", text: $password)
                    .textContentType(.newPassword)
                    .focused(isFocused)
            }

            Button(action: togglePasswordVisibility) {
                Image(systemName: isPasswordVisible ? "eye.slash" : "eye")
                    .foregroundColor(.secondary)
            }
        }
    }

    private func togglePasswordVisibility() {
        isPasswordVisible.toggle()
    }
}

private struct BackupPasswordActionButtons: View {
    let canGenerate: Bool
    let canCopy: Bool
    let isManualPasswordEntryVisible: Bool
    let actions: BackupPasswordActions

    var body: some View {
        if canGenerate {
            Button(action: actions.generate) {
                Label("Generate 12-Word Password", systemImage: "key.horizontal")
            }
        }

        Button(action: actions.retrieve) {
            Label("Retrieve from Password Manager", systemImage: "key.fill")
        }

        Button(action: actions.toggleManualEntry) {
            Label(
                isManualPasswordEntryVisible ? "Hide Manual Password Entry" : "Enter Password Manually",
                systemImage: "keyboard"
            )
        }

        if canCopy {
            Button(action: actions.copy) {
                Label("Copy Password", systemImage: "doc.on.doc")
            }
        }
    }
}

private struct BackupPasswordSectionHeader: View {
    let hasGeneratedPassword: Bool

    var body: some View {
        if !hasGeneratedPassword {
            Text("Backup Password")
        }
    }
}

private struct BackupPasswordSectionFooter: View {
    let password: String
    let isPasswordValid: Bool
    let hasGeneratedPassword: Bool

    var body: some View {
        if !password.isEmpty, !isPasswordValid, !hasGeneratedPassword {
            Text("Password must be at least 20 characters")
                .foregroundColor(.red)
        }
    }
}

private struct BackupSecurityWarningSection: View {
    var body: some View {
        Section {
            Label {
                Text("This backup contains all your wallet private keys. Keep the file and password secure.")
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundColor(.orange)
            }
        }
    }
}

private struct BackupExportActionSection: View {
    let isExporting: Bool
    let passwordSetupOptions: BackupPasswordSetupOptions
    let export: () -> Void

    var body: some View {
        Section {
            Button(action: export) {
                if isExporting {
                    HStack {
                        Spacer()
                        ProgressView()
                        Spacer()
                    }
                } else {
                    HStack {
                        Spacer()
                        Text("Export Backup")
                            .fontWeight(.semibold)
                        Spacer()
                    }
                }
            }
            .disabled(isExporting)
            .confirmationDialog(
                "Choose Backup Password",
                isPresented: passwordSetupOptions.isPresented,
                titleVisibility: .visible
            ) {
                Button("Generate 12-Word Password", action: passwordSetupOptions.generate)
                Button("Retrieve from Password Manager", action: passwordSetupOptions.retrieve)
                Button("Enter Password Manually", action: passwordSetupOptions.enterManually)
                Button("Cancel", role: .cancel, action: passwordSetupOptions.cancel)
            } message: {
                Text("Choose how you want to provide the backup password before exporting.")
            }
        }
    }
}

private struct ExportConfirmationAlert<Content: View>: View {
    @Binding var isPresented: Bool

    let export: () -> Void
    let content: Content

    var body: some View {
        content.alert("Export Backup?", isPresented: $isPresented) {
            Button("Export", role: .destructive, action: export)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This backup will contain all your wallet private keys. Make sure you keep the file and password secure.")
        }
    }
}

private struct ExportFailureAlert<Content: View>: View {
    @Binding var message: String?
    let content: Content

    var isPresented: Binding<Bool> {
        Binding(
            get: { message != nil },
            set: { if !$0 { message = nil } }
        )
    }

    var body: some View {
        content.alert("Export Failed", isPresented: isPresented) {
            Button("OK", action: clearMessage)
        } message: {
            Text(message ?? "Unknown error")
        }
    }

    private func clearMessage() {
        message = nil
    }
}

private struct ExportWarningAlert<Content: View>: View {
    @Binding var message: String?
    let content: Content

    var isPresented: Binding<Bool> {
        Binding(
            get: { message != nil },
            set: { if !$0 { message = nil } }
        )
    }

    var body: some View {
        content.alert("Export Warnings", isPresented: isPresented) {
            Button("OK", action: clearMessage)
        } message: {
            Text(message ?? "")
        }
    }

    private func clearMessage() {
        message = nil
    }
}

private struct SaveToPasswordsAlert<Content: View>: View {
    @Binding var isPresented: Bool

    let save: () -> Void
    let skip: () -> Void
    let content: Content

    var body: some View {
        content.alert("Save to Apple Passwords?", isPresented: $isPresented) {
            Button("Save", action: save)
            Button("Skip", role: .cancel, action: skip)
        } message: {
            Text("Save the backup password to Apple Passwords so you can retrieve it later during import. If you use a third-party password manager, copy the password and save it manually instead.")
        }
    }
}

private struct BackupPasswordSetupOptions {
    let isPresented: Binding<Bool>
    let generate: () -> Void
    let retrieve: () -> Void
    let enterManually: () -> Void
    let cancel: () -> Void
}

private struct PasswordCopiedAlert<Content: View>: View {
    @Binding var message: String?

    let continueExport: () -> Void
    let content: Content

    var isPresented: Binding<Bool> {
        Binding(
            get: { message != nil },
            set: handlePresentationChange
        )
    }

    var body: some View {
        content.alert("Password Copied", isPresented: isPresented) {
            Button("OK", action: clearMessage)
        } message: {
            Text(message ?? "")
        }
    }

    private func handlePresentationChange(_ isPresented: Bool) {
        guard !isPresented else { return }

        message = nil
        continueExport()
    }

    private func clearMessage() {
        message = nil
    }
}
