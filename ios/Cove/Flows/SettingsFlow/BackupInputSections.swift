import SwiftUI

struct BackupFileSelectionSection: View {
    let fileName: String?
    let hasFile: Bool
    let onSelectFile: () -> Void

    var body: some View {
        Section("Backup File") {
            Button(action: onSelectFile) {
                HStack {
                    Image(systemName: "doc.badge.plus")
                    Text(fileName ?? "Select Backup File")
                    Spacer()

                    if hasFile {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundColor(.green)
                    }
                }
            }
        }
    }
}

struct BackupCredentialsSections: View {
    let isPresented: Bool
    let isPasswordValid: Bool
    let actionTitle: String
    let isRunning: Bool
    let onRetrievePassword: () -> Void
    let action: () -> Void

    @Binding var password: String
    @Binding var isPasswordVisible: Bool

    init(
        isPresented: Bool,
        password: Binding<String>,
        isPasswordVisible: Binding<Bool>,
        isPasswordValid: Bool,
        actionTitle: String,
        isRunning: Bool,
        onRetrievePassword: @escaping () -> Void,
        action: @escaping () -> Void
    ) {
        self.isPresented = isPresented
        self.isPasswordValid = isPasswordValid
        self.actionTitle = actionTitle
        self.isRunning = isRunning
        self.onRetrievePassword = onRetrievePassword
        self.action = action
        _password = password
        _isPasswordVisible = isPasswordVisible
    }

    var body: some View {
        if isPresented {
            BackupPasswordSection(
                isPasswordValid: isPasswordValid,
                onRetrievePassword: onRetrievePassword,
                password: $password,
                isPasswordVisible: $isPasswordVisible
            )
            BackupProgressActionSection(
                title: actionTitle,
                isRunning: isRunning,
                isEnabled: isPasswordValid && !isRunning,
                action: action
            )
        }
    }
}

private struct BackupPasswordSection: View {
    let isPasswordValid: Bool
    let onRetrievePassword: () -> Void

    @Binding var password: String
    @Binding var isPasswordVisible: Bool

    var body: some View {
        Section {
            BackupPasswordField(
                password: $password,
                isPasswordVisible: $isPasswordVisible
            )

            Button(action: onRetrievePassword) {
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
    }
}

private struct BackupPasswordField: View {
    @Binding var password: String
    @Binding var isPasswordVisible: Bool

    var body: some View {
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

private struct BackupProgressActionSection: View {
    let title: String
    let isRunning: Bool
    let isEnabled: Bool
    let action: () -> Void

    var body: some View {
        Section {
            Button(action: action) {
                BackupProgressButtonLabel(
                    title: title,
                    isRunning: isRunning
                )
            }
            .disabled(!isEnabled)
        }
    }
}

struct BackupProgressButtonLabel: View {
    let title: String
    let isRunning: Bool

    var body: some View {
        if isRunning {
            HStack {
                Spacer()
                ProgressView()
                Spacer()
            }
        } else {
            HStack {
                Spacer()
                Text(title)
                    .fontWeight(.semibold)
                Spacer()
            }
        }
    }
}
