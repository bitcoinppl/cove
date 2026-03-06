import AuthenticationServices
import Security
import SwiftUI

struct BackupExportView: View {
    @Environment(\.dismiss) private var dismiss
    @FocusState private var isManualPasswordFieldFocused: Bool

    @State private var password = ""
    @State private var generatedPassword: String? = nil
    @State private var isPasswordVisible = false
    @State private var isManualPasswordEntryVisible = false
    @State private var isExporting = false
    @State private var showConfirmation = false
    @State private var showPasswordSetupOptions = false
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

    private var isPasswordValid: Bool {
        backupManager.isPasswordValid(password: password)
    }

    var body: some View {
        Form {
            if let generated = generatedPassword {
                Section {
                    Text(generated)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)

                    Button(action: {
                        UIPasteboard.general.setItems(
                            [["public.utf8-plain-text": generated]],
                            options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(120)]
                        )
                    }) {
                        Label("Copy Password", systemImage: "doc.on.doc")
                    }

                    Button(action: saveToPasswords) {
                        Label("Save to Apple Passwords", systemImage: "lock.shield")
                    }

                    Button(role: .destructive, action: clearGeneratedPassword) {
                        Label("Clear", systemImage: "xmark.circle")
                    }
                } header: {
                    Text("Generated Backup Password")
                } footer: {
                    Text("Using a third-party password manager? Copy the password and save it manually")
                }
            }

            Section {
                if isManualPasswordEntryVisible {
                    HStack {
                        if isPasswordVisible {
                            TextField("Password", text: $password)
                                .autocorrectionDisabled()
                                .textInputAutocapitalization(.never)
                                .textContentType(.newPassword)
                                .focused($isManualPasswordFieldFocused)
                        } else {
                            SecureField("Password", text: $password)
                                .textContentType(.newPassword)
                                .focused($isManualPasswordFieldFocused)
                        }
                        Button(action: { isPasswordVisible.toggle() }) {
                            Image(systemName: isPasswordVisible ? "eye.slash" : "eye")
                                .foregroundColor(.secondary)
                        }
                    }
                }

                if generatedPassword == nil, !password.isEmpty {
                    Label(passwordStatusText, systemImage: "checkmark.circle.fill")
                        .foregroundColor(.secondary)
                }

                if generatedPassword == nil {
                    Button(action: prepareGeneratedPassword) {
                        Label("Generate 12-Word Password", systemImage: "key.horizontal")
                    }
                }

                Button(action: retrieveFromPasswords) {
                    Label("Retrieve from Password Manager", systemImage: "key.fill")
                }

                Button(action: showManualPasswordEntry) {
                    Label(isManualPasswordEntryVisible ? "Hide Manual Password Entry" : "Enter Password Manually", systemImage: "keyboard")
                }

                if generatedPassword == nil, !password.isEmpty {
                    Button(action: {
                        UIPasteboard.general.setItems(
                            [["public.utf8-plain-text": password]],
                            options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(120)]
                        )
                    }) {
                        Label("Copy Password", systemImage: "doc.on.doc")
                    }
                }
            } header: {
                if generatedPassword == nil {
                    Text("Backup Password")
                }
            } footer: {
                if !password.isEmpty, !isPasswordValid, generatedPassword == nil {
                    Text("Password must be at least 20 characters")
                        .foregroundColor(.red)
                }
            }

            Section {
                Label {
                    Text("This backup contains all your wallet private keys. Keep the file and password secure.")
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundColor(.orange)
                }
            }

            Section {
                Button(action: handleExportTapped) {
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
            }
        }
        .onDisappear {
            exportTask?.cancel()
            password = ""
            if let url = tempFileURL {
                do {
                    try FileManager.default.removeItem(at: url)
                } catch {
                    print("Warning: failed to delete temp backup file: \(error.localizedDescription)")
                }
                tempFileURL = nil
            }
        }
        .alert("Export Backup?", isPresented: $showConfirmation) {
            Button("Export", role: .destructive) { exportBackup() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This backup will contain all your wallet private keys. Make sure you keep the file and password secure.")
        }
        .alert("Export Failed", isPresented: .init(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )) {
            Button("OK") { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "Unknown error")
        }
        .alert("Export Warnings", isPresented: .init(
            get: { warningMessage != nil },
            set: { if !$0 { warningMessage = nil } }
        )) {
            Button("OK") { warningMessage = nil }
        } message: {
            Text(warningMessage ?? "")
        }
        .alert("Save to Apple Passwords?", isPresented: $showSaveToPasswords) {
            Button("Save") {
                saveToPasswords()
            }
            Button("Skip", role: .cancel) {
                continuePendingExportIfReady()
            }
        } message: {
            Text("Save the backup password to Apple Passwords so you can retrieve it later during import. If you use a third-party password manager, copy the password and save it manually instead.")
        }
        .confirmationDialog("Choose Backup Password", isPresented: $showPasswordSetupOptions, titleVisibility: .visible) {
            Button("Generate 12-Word Password") {
                prepareGeneratedPassword()
            }
            Button("Retrieve from Password Manager") {
                retrieveFromPasswords()
            }
            Button("Enter Password Manually") {
                showManualPasswordEntry()
            }
            Button("Cancel", role: .cancel) {
                pendingExportAfterPasswordSetup = false
            }
        } message: {
            Text("Choose how you want to provide the backup password before exporting.")
        }
        .alert("Password Copied", isPresented: .init(
            get: { passwordCopiedMessage != nil },
            set: {
                if !$0 {
                    passwordCopiedMessage = nil
                    continuePendingExportIfReady()
                }
            }
        )) {
            Button("OK") { passwordCopiedMessage = nil }
        } message: {
            Text(passwordCopiedMessage ?? "")
        }
    }

    private var passwordStatusText: String {
        if isManualPasswordEntryVisible {
            return "Manual backup password ready"
        }

        return "Backup password ready"
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
                showPasswordSetupOptions = true
            } else {
                showManualPasswordEntry()
            }
            return
        }

        showConfirmation = true
    }

    private func continuePendingExportIfReady() {
        guard pendingExportAfterPasswordSetup, isPasswordValid else { return }

        pendingExportAfterPasswordSetup = false
        Task { @MainActor in
            await Task.yield()
            showConfirmation = true
        }
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
                            print("Warning: failed to delete temp backup file: \(error.localizedDescription)")
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
