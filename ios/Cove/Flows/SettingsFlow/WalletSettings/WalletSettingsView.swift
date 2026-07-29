import SwiftUI

struct WalletSettingsView: View {
    @Environment(AppManager.self) private var app
    @Environment(\.navigate) private var navigate
    @Environment(\.dismiss) private var dismiss

    let manager: WalletManager

    @State private var cloudBackupManager = CloudBackupManager.shared
    @State private var showingDeleteConfirmation = false
    @State private var showingSecretWordsConfirmation = false
    @State private var showingSecondDeleteConfirmation = false
    @State private var showingFinalDeleteConfirmation = false
    @State private var requiredConfirmations: UInt8 = 1
    @State private var accountNumber: UInt32? = nil

    init(manager: WalletManager) {
        self.manager = manager
    }

    var metadata: WalletMetadata {
        manager.walletMetadata
    }

    var deleteConfirmationMessage: String {
        manager.rust.deletionWarningMessage()
    }

    var finalDeleteConfirmationMessage: String {
        if cloudBackupManager.isCloudBackupEnabled {
            return "This wallet will be deleted from this device. You can recover it from the Cloud Backup screen, or permanently delete it from there."
        }

        return "This wallet is not backed up and contains funds. You will lose access to these funds forever."
    }

    var finalDeleteButtonTitle: String {
        cloudBackupManager.isCloudBackupEnabled ? "Delete" : "Delete Forever"
    }

    private var showLabels: Binding<Bool> {
        Binding(
            get: { manager.walletMetadata.showLabels },
            set: { _ in manager.dispatch(action: .toggleShowLabels) }
        )
    }

    let colorColumns = Array(repeating: GridItem(.flexible(), spacing: 0), count: 5)

    private func deleteWallet() {
        do {
            try manager.rust.deleteWallet()
            dismiss()
        } catch {
            Log.error("Unable to delete wallet: \(error)")
        }
    }

    private func changeName() {
        app.pushRoute(Route.settings(.wallet(id: metadata.id, route: .changeName)))
    }

    private func updateColor(_ color: WalletColor) {
        manager.dispatch(action: .updateColor(color))
    }

    private func showSecretWords() {
        app.pushRoute(Route.secretWords(manager.walletMetadata.id))
    }

    private func prepareDelete() {
        requiredConfirmations = manager.rust.requiredDeletionConfirmations()
        showingDeleteConfirmation = true
    }

    var body: some View {
        List {
            WalletSettingsInformationSection(
                metadata: metadata,
                accountNumber: accountNumber,
                masterFingerprint: manager.rust.masterFingerprint()
            )
            WalletSettingsPreferencesSection(
                metadata: metadata,
                colorColumns: colorColumns,
                showLabels: showLabels,
                changeName: changeName,
                updateColor: updateColor
            )
            WalletSettingsDangerSection(
                walletName: metadata.name,
                isHotWallet: metadata.walletType == .hot,
                deleteConfirmationMessage: deleteConfirmationMessage,
                finalDeleteConfirmationMessage: finalDeleteConfirmationMessage,
                finalDeleteButtonTitle: finalDeleteButtonTitle,
                requiredConfirmations: $requiredConfirmations,
                showingSecretWordsConfirmation: $showingSecretWordsConfirmation,
                showingDeleteConfirmation: $showingDeleteConfirmation,
                showingSecondDeleteConfirmation: $showingSecondDeleteConfirmation,
                showingFinalDeleteConfirmation: $showingFinalDeleteConfirmation,
                showSecretWords: showSecretWords,
                prepareDelete: prepareDelete,
                deleteWallet: deleteWallet
            )
        }
        .navigationTitle(manager.walletMetadata.name)
        .navigationBarTitleDisplayMode(.inline)
        .foregroundColor(.primary)
        .onDisappear { manager.validateMetadata() }
        .onAppear { manager.validateMetadata() }
        .task {
            accountNumber = manager.rust.nonDefaultAccountNumber()
        }
        .scrollContentBackground(.hidden)
    }
}

private struct WalletSettingsInformationSection: View {
    let metadata: WalletMetadata
    let accountNumber: UInt32?
    let masterFingerprint: String?

    var body: some View {
        Section(header: Text("Wallet Information")) {
            WalletSettingsValueRow(title: "Network", value: metadata.network.description)

            if let birthday = metadata.birthday {
                WalletSettingsValueRow(title: "Birthday", value: birthday.displayValue)
            }

            if let accountNumber {
                WalletSettingsValueRow(title: "Account Number", value: "\(accountNumber)")
            }

            if let masterFingerprint, !metadata.isTapSigner() {
                WalletSettingsValueRow(title: "Fingerprint", value: masterFingerprint)
            }

            if case let .tapSigner(tapSigner) = metadata.hardwareMetadata {
                WalletSettingsValueRow(
                    title: "Card Identifier",
                    value: tapSigner.fullCardIdent(),
                    minimumScaleFactor: 0.75
                )
            }

            WalletSettingsValueRow(title: "Wallet Type", value: String(metadata.walletType))
        }
    }
}

private struct WalletSettingsValueRow: View {
    let title: String
    let value: String
    var minimumScaleFactor: CGFloat = 1

    var body: some View {
        HStack {
            Text(title)
            Spacer()
            Text(value)
                .foregroundColor(.secondary)
                .minimumScaleFactor(minimumScaleFactor)
        }
        .font(.subheadline)
    }
}

private struct WalletSettingsPreferencesSection: View {
    let metadata: WalletMetadata
    let colorColumns: [GridItem]
    @Binding var showLabels: Bool
    let changeName: () -> Void
    let updateColor: (WalletColor) -> Void

    var body: some View {
        Section(header: Text("Settings")) {
            WalletSettingsNameRow(name: metadata.name, changeName: changeName)
            WalletSettingsColorPicker(
                selectedColor: metadata.color,
                colorColumns: colorColumns,
                updateColor: updateColor
            )
            Toggle(isOn: $showLabels) {
                Text("Show transaction labels")
                    .font(.subheadline)
            }
            .padding(.vertical, 1)
        }
    }
}

private struct WalletSettingsNameRow: View {
    let name: String
    let changeName: () -> Void

    var body: some View {
        HStack {
            Text("Name")
            Spacer()
            Text(name)
                .font(.subheadline)
                .foregroundColor(.secondary)
            Image(systemName: "chevron.right")
                .foregroundColor(Color(UIColor.tertiaryLabel))
                .font(.footnote)
                .fontWeight(.semibold)
        }
        .contentShape(Rectangle())
        .font(.subheadline)
        .onTapGesture(perform: changeName)
    }
}

private struct WalletSettingsColorPicker: View {
    let selectedColor: WalletColor
    let colorColumns: [GridItem]
    let updateColor: (WalletColor) -> Void

    var body: some View {
        VStack(spacing: 14) {
            HStack {
                Text("Wallet Color")
                    .font(.subheadline)
                Spacer()
            }
            HStack {
                Rectangle()
                    .fill(Color(selectedColor))
                    .cornerRadius(10)
                    .frame(width: 80, height: 80)
                LazyVGrid(columns: colorColumns, spacing: 20) {
                    ForEach(defaultWalletColors(), id: \.self) { color in
                        WalletSettingsColorButton(
                            color: color,
                            isSelected: color == selectedColor,
                            select: { updateColor(color) }
                        )
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .frame(maxWidth: .infinity)
            }
        }
        .padding(.vertical, 8)
    }
}

private struct WalletSettingsColorButton: View {
    let color: WalletColor
    let isSelected: Bool
    let select: () -> Void

    var body: some View {
        ZStack {
            if isSelected {
                Circle()
                    .stroke(Color(color).opacity(0.7), lineWidth: 2)
                    .frame(width: 32, height: 32)
            }
            Circle()
                .fill(Color(color))
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
        }
        .onTapGesture(perform: select)
    }
}

private struct WalletSettingsDangerSection: View {
    let walletName: String
    let isHotWallet: Bool
    let deleteConfirmationMessage: String
    let finalDeleteConfirmationMessage: String
    let finalDeleteButtonTitle: String
    @Binding var requiredConfirmations: UInt8
    @Binding var showingSecretWordsConfirmation: Bool
    @Binding var showingDeleteConfirmation: Bool
    @Binding var showingSecondDeleteConfirmation: Bool
    @Binding var showingFinalDeleteConfirmation: Bool
    let showSecretWords: () -> Void
    let prepareDelete: () -> Void
    let deleteWallet: () -> Void

    var body: some View {
        Section(header: Text("Danger Zone")) {
            if isHotWallet {
                WalletSecretWordsButton(
                    isPresented: $showingSecretWordsConfirmation,
                    showSecretWords: showSecretWords
                )
            }
            WalletFinalDeleteConfirmationHost(
                isPresented: $showingFinalDeleteConfirmation,
                buttonTitle: finalDeleteButtonTitle,
                message: finalDeleteConfirmationMessage,
                delete: deleteWallet
            ) {
                WalletSecondDeleteConfirmationHost(
                    isPresented: $showingSecondDeleteConfirmation,
                    requiredConfirmations: requiredConfirmations,
                    walletName: walletName,
                    showFinalConfirmation: { showingFinalDeleteConfirmation = true },
                    delete: deleteWallet
                ) {
                    WalletInitialDeleteConfirmationHost(
                        isPresented: $showingDeleteConfirmation,
                        requiredConfirmations: requiredConfirmations,
                        message: deleteConfirmationMessage,
                        showSecondConfirmation: { showingSecondDeleteConfirmation = true },
                        delete: deleteWallet
                    ) {
                        WalletDeleteButton(action: prepareDelete)
                    }
                }
            }
        }
    }
}

private struct WalletSecretWordsButton: View {
    @Binding var isPresented: Bool
    let showSecretWords: () -> Void

    var body: some View {
        Button {
            isPresented = true
        } label: {
            Text("View Secret Words")
                .font(.subheadline)
        }
        .confirmationDialog("Are you sure?", isPresented: $isPresented) {
            Button("Show Me", action: showSecretWords)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "Whoever has access to your secret words, has access to your bitcoin. Please keep these safe, don't show them to anyone."
            )
        }
    }
}

private struct WalletDeleteButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text("Delete Wallet")
                .foregroundStyle(.red)
                .font(.subheadline)
        }
    }
}

private struct WalletInitialDeleteConfirmationHost<Content: View>: View {
    @Binding var isPresented: Bool
    let requiredConfirmations: UInt8
    let message: String
    let showSecondConfirmation: () -> Void
    let delete: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .confirmationDialog("Are you sure?", isPresented: $isPresented) {
                Button("Delete", role: .destructive, action: confirmDelete)
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(message)
            }
    }

    private func confirmDelete() {
        if requiredConfirmations >= 2 {
            showSecondConfirmation()
        } else {
            delete()
        }
    }
}

private struct WalletSecondDeleteConfirmationHost<Content: View>: View {
    @Binding var isPresented: Bool
    let requiredConfirmations: UInt8
    let walletName: String
    let showFinalConfirmation: () -> Void
    let delete: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .alert("Confirm Deletion", isPresented: $isPresented) {
                Button("Delete", role: .destructive, action: confirmDelete)
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Are you sure you want to delete '\(walletName)'?")
            }
    }

    private func confirmDelete() {
        if requiredConfirmations >= 3 {
            showFinalConfirmation()
        } else {
            delete()
        }
    }
}

private struct WalletFinalDeleteConfirmationHost<Content: View>: View {
    @Binding var isPresented: Bool
    let buttonTitle: String
    let message: String
    let delete: () -> Void
    @ViewBuilder let content: Content

    var body: some View {
        content
            .alert("Final Warning", isPresented: $isPresented) {
                Button(buttonTitle, role: .destructive, action: delete)
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(message)
            }
    }
}

private extension WalletBirthday {
    var displayValue: String {
        switch self {
        case .blockHeight:
            "Block \(blockHeightFmt() ?? "")"
        case let .timestamp(timestamp):
            Date(timeIntervalSince1970: TimeInterval(timestamp))
                .formatted(date: .abbreviated, time: .omitted)
        }
    }
}

#Preview {
    AsyncPreview {
        WalletSettingsView(manager: WalletManager(preview: "preview_only"))
            .environment(AppManager.shared)
            .environment(\.navigate) { _ in
                ()
            }
            .background(Color(UIColor.systemGroupedBackground))
    }
}
