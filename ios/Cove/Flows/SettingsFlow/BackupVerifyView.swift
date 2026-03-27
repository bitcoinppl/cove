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

    private var isPasswordValid: Bool {
        backupManager.isPasswordValid(password: password)
    }

    var body: some View {
        Form {
            if let report = verifyReport {
                VerifyResultView(report: report)
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
                                    Text("Verify Backup")
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
            verifyTask?.cancel()
            password = ""
            fileData = nil
        }
        .fileImporter(
            isPresented: $showFilePicker,
            allowedContentTypes: [.data],
            onCompletion: handleFileSelection
        )
        .alert("Verification Failed", isPresented: .init(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )) {
            Button("OK") { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "Unknown error")
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

    @State private var passwordDelegate: PasswordRetrievalDelegate? = nil
}

struct VerifyResultView: View {
    let report: BackupVerifyReport

    var body: some View {
        Section {
            HStack {
                Image(systemName: "checkmark.shield.fill")
                    .foregroundColor(.green)
                    .font(.title2)
                Text("Backup Verified Successfully")
                    .fontWeight(.semibold)
            }
        }

        Section("Backup Info") {
            LabeledContent("Created", value: formatDate(report.createdAt))
            LabeledContent("Wallets", value: "\(report.walletCount)")
        }

        ForEach(Array(report.wallets.enumerated()), id: \.offset) { _, wallet in
            Section {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text(wallet.name)
                            .fontWeight(.medium)
                        Spacer()
                        Text(wallet.alreadyOnDevice ? "Already on device" : "New")
                            .font(.caption)
                            .fontWeight(.medium)
                            .foregroundColor(wallet.alreadyOnDevice ? .secondary : .green)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(
                                wallet.alreadyOnDevice
                                    ? Color.secondary.opacity(0.15)
                                    : Color.green.opacity(0.15),
                                in: Capsule()
                            )
                    }

                    Divider()

                    Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 10) {
                        GridRow {
                            IconLabel("globe", wallet.network.displayName())
                            IconLabel("wallet.bifold", wallet.walletType.displayName())
                        }

                        GridRow {
                            if let fingerprint = wallet.fingerprint {
                                IconLabel("touchid", fingerprint)
                            } else {
                                Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
                            }
                            IconLabel("key", wallet.secretType.displayName())
                        }

                        if wallet.labelCount > 0 {
                            GridRow {
                                IconLabel("tag", "\(wallet.labelCount) labels")
                                Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
                            }
                        }
                    }
                    .font(.caption)
                    .foregroundColor(.secondary)

                    if let warning = wallet.warning {
                        Label(warning, systemImage: "exclamationmark.triangle.fill")
                            .font(.caption)
                            .foregroundColor(.orange)
                    }
                }
                .padding(.vertical, 4)
            }
        }

        Section("Settings") {
            if let fiat = report.fiatCurrency {
                LabeledContent("Fiat Currency", value: fiat)
            }
            if let scheme = report.colorScheme {
                LabeledContent("Color Scheme", value: scheme)
            }
            LabeledContent("Node Configs", value: "\(report.nodeConfigCount)")
        }
    }

    private func formatDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

struct IconLabel: View {
    let icon: String
    let text: String

    init(_ icon: String, _ text: String) {
        self.icon = icon
        self.text = text
    }

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: icon)
                .frame(width: 14)
            Text(text)
        }
    }
}
