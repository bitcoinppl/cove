import AuthenticationServices
import SwiftUI
import UniformTypeIdentifiers

struct BackupImportView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss
    let onImported: (() -> Void)?

    init(onImported: (() -> Void)? = nil) {
        self.onImported = onImported
    }

    @State private var fileData: Data? = nil
    @State private var fileName: String? = nil
    @State private var password = ""
    @State private var isPasswordVisible = false
    @State private var isImporting = false
    @State private var isVerifying = false
    @State private var showFilePicker = false
    @State private var errorMessage: String? = nil
    @State private var verifyReport: BackupVerifyReport? = nil
    @State private var importReport: BackupImportReport? = nil
    @State private var importTask: Task<Void, Never>? = nil
    @State private var showConfirmation = false

    @State private var backupManager = BackupManager()

    private var isPasswordValid: Bool {
        backupManager.isPasswordValid(password: password)
    }

    var body: some View {
        Form {
            if let report = verifyReport {
                VerifyResultView(report: report)

                Section {
                    Button(action: { showConfirmation = true }) {
                        if isImporting {
                            HStack {
                                Spacer()
                                ProgressView()
                                Spacer()
                            }
                        } else {
                            HStack {
                                Spacer()
                                Text("Confirm Import")
                                    .fontWeight(.semibold)
                                Spacer()
                            }
                        }
                    }
                    .disabled(isImporting)

                    Button(action: { verifyReport = nil }) {
                        HStack {
                            Spacer()
                            Text("Back")
                            Spacer()
                        }
                    }
                }
            } else {
                Section("Backup File") {
                    Button(action: { showFilePicker = true }) {
                        HStack {
                            Image(systemName: "doc.badge.plus")
                            Text(fileName ?? "Select Backup File")
                            Spacer()
                            if fileData != nil {
                                Image(systemName: "checkmark.circle.fill")
                                    .foregroundColor(.green)
                            }
                        }
                    }
                }

                if fileData != nil {
                    Section {
                        HStack {
                            if isPasswordVisible {
                                TextField("Password", text: $password)
                                    .autocorrectionDisabled()
                                    .textInputAutocapitalization(.never)
                                    .textContentType(.password)
                            } else {
                                SecureField("Password", text: $password)
                                    .textContentType(.password)
                            }
                            Button(action: { isPasswordVisible.toggle() }) {
                                Image(systemName: isPasswordVisible ? "eye.slash" : "eye")
                                    .foregroundColor(.secondary)
                            }
                        }

                        Button(action: retrieveFromPasswords) {
                            Label("Retrieve from Password Manager", systemImage: "key.fill")
                        }
                    } header: {
                        Text("Backup Password")
                    } footer: {
                        if !password.isEmpty, !isPasswordValid {
                            Text("Password must be at least 20 characters")
                                .foregroundColor(.red)
                        }
                    }

                    Section {
                        Button(action: verifyBackup) {
                            if isVerifying {
                                HStack {
                                    Spacer()
                                    ProgressView()
                                    Spacer()
                                }
                            } else {
                                HStack {
                                    Spacer()
                                    Text("Preview Backup")
                                        .fontWeight(.semibold)
                                    Spacer()
                                }
                            }
                        }
                        .disabled(!isPasswordValid || isVerifying)
                    }
                }
            }
        }
        .onDisappear {
            importTask?.cancel()
            password = ""
            fileData = nil
            verifyReport = nil
        }
        .alert("Import Backup?", isPresented: $showConfirmation) {
            Button("Import", role: .destructive) { importBackup() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This will import wallets and restore settings from the backup. Existing wallets with the same fingerprint will be skipped.")
        }
        .fileImporter(
            isPresented: $showFilePicker,
            // no registered UTType for .covb — downstream validation handles invalid files
            allowedContentTypes: [.data],
            onCompletion: handleFileSelection
        )
        .alert("Import Failed", isPresented: .init(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )) {
            Button("OK") { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "Unknown error")
        }
        .alert("Import Complete", isPresented: .init(
            get: { importReport != nil },
            set: { if !$0 {
                handleImportCompletionDismissal()
            }}
        )) {
            Button("OK") {
                handleImportCompletionDismissal()
            }
        } message: {
            if let report = importReport {
                Text(formatReport(report))
            }
        }
    }

    private func handleFileSelection(_ result: Result<URL, Error>) {
        switch result {
        case let .success(url):
            guard url.startAccessingSecurityScopedResource() else {
                errorMessage = "Unable to access the selected file"
                return
            }
            defer { url.stopAccessingSecurityScopedResource() }

            do {
                let attrs = try url.resourceValues(forKeys: [.fileSizeKey])
                if let size = attrs.fileSize, size > 50_000_000 {
                    throw BackupError.FileTooLarge
                }

                let data = try Data(contentsOf: url)
                if data.count > 50_000_000 {
                    throw BackupError.FileTooLarge
                }
                try backupManager.validateFormat(data: data)

                fileData = data
                fileName = url.lastPathComponent
            } catch {
                fileData = nil
                fileName = nil
                errorMessage = (error as? BackupError)?.description ?? error.localizedDescription
            }

        case let .failure(error):
            errorMessage = error.localizedDescription
        }
    }

    private func verifyBackup() {
        guard let fileData else { return }
        isVerifying = true
        importTask = Task {
            do {
                let report = try await backupManager.verifyBackup(data: fileData, password: password)
                await MainActor.run {
                    isVerifying = false
                    verifyReport = report
                }
            } catch {
                await MainActor.run {
                    isVerifying = false
                    errorMessage = (error as? BackupError)?.description ?? error.localizedDescription
                }
            }
        }
    }

    private func importBackup() {
        guard let fileData else { return }
        isImporting = true
        importTask = Task {
            do {
                let report = try await backupManager.importBackup(data: fileData, password: password)
                await MainActor.run {
                    isImporting = false
                    importReport = report
                }
            } catch {
                await MainActor.run {
                    isImporting = false
                    errorMessage = (error as? BackupError)?.description ?? error.localizedDescription
                }
            }
        }
    }

    private func handleImportCompletionDismissal() {
        importReport = nil
        app.dispatch(action: .refreshAfterImport)
        if let onImported {
            onImported()
        } else {
            dismiss()
        }
    }

    private func retrieveFromPasswords() {
        let provider = ASAuthorizationPasswordProvider()
        let request = provider.createRequest()
        let controller = ASAuthorizationController(authorizationRequests: [request])
        let delegate = PasswordRetrievalDelegate(
            onPassword: { retrievedPassword in password = retrievedPassword },
            onError: { msg in errorMessage = msg }
        )
        passwordDelegate = delegate
        controller.delegate = delegate
        controller.performRequests()
    }

    @State private var passwordDelegate: PasswordRetrievalDelegate? = nil

    private func formatReport(_ report: BackupImportReport) -> String {
        var lines: [String] = []
        lines.append("\(report.walletsImported) wallet(s) imported")
        if report.walletsSkipped > 0 {
            lines.append("\(report.walletsSkipped) wallet(s) skipped: \(report.skippedWalletNames.joined(separator: ", "))")
        }
        if report.walletsFailed > 0 {
            lines.append("\(report.walletsFailed) wallet(s) failed: \(report.failedWalletNames.joined(separator: ", "))")
        }
        if report.walletsWithLabelsImported > 0 {
            lines.append("\(report.walletsWithLabelsImported) label set(s) imported")
        }
        if !report.labelsFailedWalletNames.isEmpty {
            let names = report.labelsFailedWalletNames.joined(separator: ", ")
            if !report.labelsFailedErrors.isEmpty {
                let errors = report.labelsFailedErrors.joined(separator: "; ")
                lines.append("Labels failed for \(names): \(errors)")
            } else {
                lines.append("Labels failed for: \(names)")
            }
        }
        if report.settingsRestored {
            lines.append("Settings restored")
        }
        if let error = report.settingsError {
            lines.append("Settings partially restored: \(error)")
        }
        if !report.degradedWalletNames.isEmpty {
            lines.append("Wallets imported with limited functionality: \(report.degradedWalletNames.joined(separator: ", "))")
        }
        if !report.cleanupWarnings.isEmpty {
            lines.append("Cleanup warnings: \(report.cleanupWarnings.joined(separator: ", "))")
        }
        return lines.joined(separator: "\n")
    }
}

class PasswordRetrievalDelegate: NSObject, ASAuthorizationControllerDelegate {
    let onPassword: (String) -> Void
    let onError: ((String) -> Void)?

    init(onPassword: @escaping (String) -> Void, onError: ((String) -> Void)? = nil) {
        self.onPassword = onPassword
        self.onError = onError
    }

    func authorizationController(controller _: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
        if let credential = authorization.credential as? ASPasswordCredential {
            DispatchQueue.main.async {
                self.onPassword(credential.password)
            }
        }
    }

    func authorizationController(controller _: ASAuthorizationController, didCompleteWithError error: Error) {
        let nsError = error as NSError
        // ASAuthorizationError.canceled = 1001, don't show error for user cancellation
        if nsError.domain == ASAuthorizationError.errorDomain, nsError.code == ASAuthorizationError.canceled.rawValue {
            return
        }
        DispatchQueue.main.async {
            self.onError?("No saved passwords found")
        }
    }
}
