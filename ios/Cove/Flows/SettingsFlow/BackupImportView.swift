import AuthenticationServices
import SwiftUI
import UniformTypeIdentifiers

struct BackupImportView: View {
    private enum ImportReview: Sendable {
        case awaitingApproval(preparation: BackupImportPreparation, conflictWalletCount: Int)
        case approved(
            preparation: BackupImportPreparation,
            approval: BackupImportApproval,
            conflictWalletCount: Int
        )

        var conflictWalletCount: Int {
            switch self {
            case let .awaitingApproval(_, count), let .approved(_, _, count):
                count
            }
        }
    }

    @Environment(AppManager.self) private var app
    @Environment(\.dismiss) private var dismiss

    let onImported: (() -> Void)?

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
    @State private var importReview: ImportReview? = nil
    @State private var importTask: Task<Void, Never>? = nil
    @State private var inputGenerations = GenerationTracker()
    @State private var showConfirmation = false
    @State private var showCleanupConfirmation = false
    @State private var backupManager = BackupManager()
    @State private var passwordDelegate: PasswordRetrievalDelegate? = nil

    private var isPasswordValid: Bool {
        backupManager.isPasswordValid(password: password)
    }

    private var isErrorPresented: Binding<Bool> {
        Binding(
            get: { errorMessage != nil },
            set: updateErrorPresentation
        )
    }

    private var isImportCompletePresented: Binding<Bool> {
        Binding(
            get: { importReport != nil },
            set: updateImportCompletionPresentation
        )
    }

    private var importReportMessage: String? {
        importReport.map(formatReport)
    }

    private var conflictWalletCount: Int {
        importReview?.conflictWalletCount ?? 0
    }

    init(onImported: (() -> Void)? = nil) {
        self.onImported = onImported
    }

    var body: some View {
        BackupImportForm(
            verifyReport: verifyReport,
            conflictWalletCount: conflictWalletCount,
            fileName: fileName,
            hasFile: fileData != nil,
            password: $password,
            isPasswordVisible: $isPasswordVisible,
            isPasswordValid: isPasswordValid,
            isImporting: isImporting,
            isVerifying: isVerifying,
            onSelectFile: presentFilePicker,
            onRetrievePassword: retrieveFromPasswords,
            onVerifyBackup: verifyBackup,
            onConfirmImport: presentImportConfirmation,
            onBack: showBackupSelection
        )
        .onDisappear(perform: handleDisappear)
        .onChange(of: fileData, initial: false, handleFileDataChange)
        .onChange(of: password, initial: false, handlePasswordChange)
        .alert("Import Backup?", isPresented: $showConfirmation) {
            Button("Continue", action: prepareImport)
            Button("Cancel", role: .cancel, action: cancelImportConfirmation)
        } message: {
            Text("This will import wallets and restore settings from the backup. Existing wallets with the same fingerprint will be skipped.")
        }
        .alert("Remove Existing Wallet Data?", isPresented: $showCleanupConfirmation) {
            Button("Approve and Import", role: .destructive, action: approveAndImport)
            Button("Cancel", role: .cancel, action: cancelCleanupConfirmation)
        } message: {
            Text(cleanupWarningMessage)
        }
        .fileImporter(
            isPresented: $showFilePicker,
            // no registered UTType for .covb — downstream validation handles invalid files
            allowedContentTypes: [.data],
            onCompletion: handleFileSelection
        )
        .alert("Import Failed", isPresented: isErrorPresented) {
            Button("OK", action: clearError)
        } message: {
            Text(errorMessage ?? "Unknown error")
        }
        .alert("Import Complete", isPresented: isImportCompletePresented) {
            Button("OK", action: handleImportCompletionDismissal)
        } message: {
            BackupImportCompletionMessage(message: importReportMessage)
        }
    }

    private func presentFilePicker() {
        showFilePicker = true
    }

    private func presentImportConfirmation() {
        showConfirmation = true
    }

    private func showBackupSelection() {
        verifyReport = nil
        invalidateImportReview(clearVerifyReport: false, cancelTask: true)
    }

    private func handleDisappear() {
        importTask?.cancel()
        invalidateImportReview(clearVerifyReport: true)
        password = ""
        fileData = nil
        fileName = nil
    }

    private func handleFileDataChange(_: Data?, _: Data?) {
        invalidateImportReview(clearVerifyReport: true, cancelTask: true)
    }

    private func handlePasswordChange(_: String, _: String) {
        invalidateImportReview(clearVerifyReport: true, cancelTask: true)
    }

    private var cleanupWarningMessage: String {
        let walletDescription = conflictWalletCount == 1 ? "1 wallet" : "\(conflictWalletCount) wallets"
        return "\(walletDescription) has existing wallet data without a matching wallet record. Approving this import will permanently remove that data, including any existing keychain items. Removed keychain items cannot be restored."
    }

    private func invalidateImportReview(clearVerifyReport: Bool, cancelTask: Bool = false) {
        _ = inputGenerations.advance()

        if cancelTask {
            importTask?.cancel()
        }

        isImporting = false
        isVerifying = false
        importReview = nil
        showCleanupConfirmation = false

        if clearVerifyReport {
            verifyReport = nil
        }
    }

    private func cancelCleanupConfirmation() {
        invalidateImportReview(clearVerifyReport: false)
    }

    private func cancelImportConfirmation() {
        invalidateImportReview(clearVerifyReport: false)
    }

    private func updateErrorPresentation(_ isPresented: Bool) {
        guard !isPresented else { return }

        errorMessage = nil
    }

    private func clearError() {
        errorMessage = nil
    }

    private func updateImportCompletionPresentation(_ isPresented: Bool) {
        guard !isPresented else { return }

        handleImportCompletionDismissal()
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
                errorMessage = safeBackupImportErrorMessage(error)
            }

        case let .failure(error):
            errorMessage = safeBackupImportErrorMessage(error)
        }
    }

    private func verifyBackup() {
        guard let fileData else { return }
        invalidateImportReview(clearVerifyReport: false, cancelTask: true)
        let generation = inputGenerations.capture()
        let requestedPassword = password
        isVerifying = true
        importTask = Task {
            do {
                let report = try await backupManager.verifyBackup(
                    data: fileData,
                    password: requestedPassword
                )
                try Task.checkCancellation()
                await MainActor.run {
                    guard inputGenerations.isCurrent(capturedToken: generation) else { return }

                    isVerifying = false
                    verifyReport = report
                }
            } catch {
                let isCancelled = Task.isCancelled
                await MainActor.run {
                    guard inputGenerations.isCurrent(capturedToken: generation) else { return }

                    isVerifying = false
                    if !isCancelled {
                        errorMessage = safeBackupImportErrorMessage(error)
                    }
                }
            }
        }
    }

    private func prepareImport() {
        guard let fileData else { return }
        invalidateImportReview(clearVerifyReport: false, cancelTask: true)
        let generation = inputGenerations.capture()
        let requestedPassword = password
        isImporting = true
        importTask = Task {
            do {
                let preparation = try await backupManager.prepareImport(
                    data: fileData,
                    password: requestedPassword
                )
                let conflictWalletCount = preparation.markerlessConflictWalletIds().count
                try Task.checkCancellation()

                if preparation.requiresImportApproval() {
                    await MainActor.run {
                        guard inputGenerations.isCurrent(capturedToken: generation) else { return }

                        importReview = .awaitingApproval(
                            preparation: preparation,
                            conflictWalletCount: conflictWalletCount
                        )
                        isImporting = false
                        showCleanupConfirmation = true
                    }
                    return
                }

                let report = try await backupManager.importPrepared(
                    preparation: preparation,
                    approval: nil
                )

                await MainActor.run {
                    guard inputGenerations.isCurrent(capturedToken: generation) else { return }

                    isImporting = false
                    invalidateImportReview(clearVerifyReport: false)
                    importReport = report
                }
            } catch {
                let isCancelled = Task.isCancelled
                await MainActor.run {
                    guard inputGenerations.isCurrent(capturedToken: generation) else { return }

                    isImporting = false
                    invalidateImportReview(clearVerifyReport: false)
                    if !isCancelled {
                        errorMessage = safeBackupImportErrorMessage(error)
                    }
                }
            }
        }
    }

    private func approveAndImport() {
        guard case let .awaitingApproval(preparation, conflictWalletCount) = importReview else {
            invalidateImportReview(clearVerifyReport: false)
            return
        }

        showCleanupConfirmation = false
        isImporting = true
        importTask = Task {
            do {
                try Task.checkCancellation()
                let approval = try backupManager.approveImport(preparation: preparation)
                await MainActor.run {
                    importReview = .approved(
                        preparation: preparation,
                        approval: approval,
                        conflictWalletCount: conflictWalletCount
                    )
                }
                try Task.checkCancellation()
                let report = try await backupManager.importPrepared(
                    preparation: preparation,
                    approval: approval
                )

                await MainActor.run {
                    isImporting = false
                    invalidateImportReview(clearVerifyReport: false)
                    importReport = report
                }
            } catch {
                let isCancelled = Task.isCancelled
                await MainActor.run {
                    isImporting = false
                    invalidateImportReview(clearVerifyReport: false)
                    if !isCancelled {
                        errorMessage = safeBackupImportErrorMessage(error)
                    }
                }
            }
        }
    }

    private func handleImportCompletionDismissal() {
        importReport = nil
        invalidateImportReview(clearVerifyReport: true)
        fileData = nil
        fileName = nil
        password = ""
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

private struct BackupImportForm: View {
    let verifyReport: BackupVerifyReport?
    let conflictWalletCount: Int
    let fileName: String?
    let hasFile: Bool
    @Binding var password: String
    @Binding var isPasswordVisible: Bool
    let isPasswordValid: Bool
    let isImporting: Bool
    let isVerifying: Bool
    let onSelectFile: () -> Void
    let onRetrievePassword: () -> Void
    let onVerifyBackup: () -> Void
    let onConfirmImport: () -> Void
    let onBack: () -> Void

    var body: some View {
        Form {
            if let verifyReport {
                if conflictWalletCount > 0 {
                    BackupImportConflictSummarySection(walletCount: conflictWalletCount)
                }

                BackupImportConfirmationContent(
                    report: verifyReport,
                    isImporting: isImporting,
                    onConfirmImport: onConfirmImport,
                    onBack: onBack
                )
            } else {
                BackupFileSelectionSection(
                    fileName: fileName,
                    hasFile: hasFile,
                    onSelectFile: onSelectFile
                )

                BackupCredentialsSections(
                    isPresented: hasFile,
                    password: $password,
                    isPasswordVisible: $isPasswordVisible,
                    isPasswordValid: isPasswordValid,
                    actionTitle: "Preview Backup",
                    isRunning: isVerifying,
                    onRetrievePassword: onRetrievePassword,
                    action: onVerifyBackup
                )
            }
        }
    }
}

private struct BackupImportConflictSummarySection: View {
    let walletCount: Int

    var body: some View {
        Section {
            Label(
                summaryText,
                systemImage: "exclamationmark.triangle.fill"
            )
            .foregroundStyle(.orange)
        } header: {
            Text("Import Review")
        } footer: {
            Text("The backup has existing wallet data without a matching wallet record.")
        }
    }

    private var summaryText: String {
        if walletCount == 1 {
            return "1 existing wallet record needs cleanup approval"
        }

        return "\(walletCount) existing wallet records need cleanup approval"
    }
}

private func safeBackupImportErrorMessage(_ error: Error) -> String {
    guard let backupError = error as? BackupError else {
        return "Cove could not import this backup. Check the file and password, then try again."
    }

    switch backupError {
    case .PasswordTooShort:
        return "The backup password is too short."
    case .DecryptionFailed:
        return "The backup password is incorrect, or the backup is damaged."
    case .InvalidFormat:
        return "The selected file is not a valid Cove backup."
    case .FileTooLarge:
        return "The backup file is too large."
    case .UnsupportedVersion, .UnsupportedPayloadVersion:
        return "This backup was created by a newer version of Cove. Update Cove and try again."
    case .Truncated:
        return "The backup file is incomplete. Select the original backup and try again."
    case .ImportApprovalStale:
        return "The existing wallet data changed while the import was waiting for approval. Review the backup again and try again."
    case .ImportApprovalRequired:
        return "This import needs approval before existing wallet data can be removed. Review the backup again and try again."
    case .ImportApprovalUsed:
        return "This import review has expired. Review the backup again and try again."
    case .InvalidWalletId:
        return "The backup contains an invalid wallet record and cannot be imported."
    case .WalletIdOccupied:
        return "A wallet changed while the import was waiting. Review the backup again and try again."
    case .Encryption, .Serialization, .Deserialization, .Gather, .Restore, .Keychain, .Database, .Decompression:
        return "Cove could not finish importing this backup. Review it and try again."
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
