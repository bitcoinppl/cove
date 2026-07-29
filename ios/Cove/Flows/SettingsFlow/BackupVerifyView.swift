import AuthenticationServices
import SwiftUI
import UniformTypeIdentifiers

struct BackupVerifyView: View {
    @Environment(\.dismiss) private var dismiss

    @State private var fileData: Data? = nil
    @State private var fileName: String? = nil
    @State private var password = ""
    @State private var isPasswordVisible = false
    @State private var isVerifying = false
    @State private var showFilePicker = false
    @State private var errorMessage: String? = nil
    @State private var verifyReport: BackupVerifyReport? = nil
    @State private var verifyTask: Task<Void, Never>? = nil
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

    var body: some View {
        Form {
            if let report = verifyReport {
                VerifyResultView(report: report)
            } else {
                BackupFileSelectionSection(
                    fileName: fileName,
                    hasFile: fileData != nil,
                    onSelectFile: presentFilePicker
                )

                BackupCredentialsSections(
                    isPresented: fileData != nil,
                    password: $password,
                    isPasswordVisible: $isPasswordVisible,
                    isPasswordValid: isPasswordValid,
                    actionTitle: "Verify Backup",
                    isRunning: isVerifying,
                    onRetrievePassword: retrieveFromPasswords,
                    action: verifyBackup
                )
            }
        }
        .onDisappear(perform: handleDisappear)
        .fileImporter(
            isPresented: $showFilePicker,
            allowedContentTypes: [.data],
            onCompletion: handleFileSelection
        )
        .alert("Verification Failed", isPresented: isErrorPresented) {
            Button("OK", action: clearError)
        } message: {
            Text(errorMessage ?? "Unknown error")
        }
    }

    private func presentFilePicker() {
        showFilePicker = true
    }

    private func handleDisappear() {
        verifyTask?.cancel()
        password = ""
        fileData = nil
    }

    private func updateErrorPresentation(_ isPresented: Bool) {
        guard !isPresented else { return }

        errorMessage = nil
    }

    private func clearError() {
        errorMessage = nil
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
        verifyTask = Task {
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
}
