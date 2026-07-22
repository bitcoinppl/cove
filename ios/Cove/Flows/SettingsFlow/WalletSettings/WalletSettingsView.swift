import SwiftUI

private enum XprvExportAction {
    case share
    case keyTeleport
}

struct WalletSettingsView: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth
    @Environment(\.navigate) private var navigate
    @Environment(\.dismiss) private var dismiss

    let manager: WalletManager

    @State private var cloudBackupManager = CloudBackupManager.shared
    @State private var showingDeleteConfirmation = false
    @State private var showingSecretWordsConfirmation = false
    @State private var showingSecondDeleteConfirmation = false
    @State private var showingFinalDeleteConfirmation = false
    @State private var showingXprvExportWarning = false
    @State private var showingXprvExportOptions = false
    @State private var pendingXprvExportAction: XprvExportAction?
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

    let colorColumns = Array(repeating: GridItem(.flexible(), spacing: 0), count: 5)

    private func deleteWallet() {
        do {
            try manager.rust.deleteWallet()
            dismiss()
        } catch {
            Log.error("Unable to delete wallet: \(error)")
        }
    }

    private func startXprvExport(_ action: XprvExportAction) {
        if auth.isAuthEnabled {
            pendingXprvExportAction = action
            auth.lock()
        } else {
            performXprvExport(action)
        }
    }

    private func performXprvExport(_ action: XprvExportAction) {
        switch action {
        case .share:
            do {
                let xprv = try manager.rust.exposeXprv()
                ShareSheet.presentFromMenu(text: xprv)
            } catch {
                Log.error("Unable to export private key: \(error)")
            }
        case .keyTeleport:
            let keyTeleportManager = app.ensureKeyTeleportManager()
            keyTeleportManager.dispatch(.startSendFromWallet(metadata.id))
            app.pushRoute(RouteFactory().keyTeleportSend())
        }
    }

    var body: some View {
        List {
            Section(header: Text("Wallet Information")) {
                HStack {
                    Text("Network")
                    Spacer()
                    Text(metadata.network.description)
                        .foregroundColor(.secondary)
                }
                .font(.subheadline)

                if let birthday = metadata.birthday {
                    HStack {
                        Text("Birthday")
                        Spacer()
                        Text(birthday.displayValue)
                            .foregroundColor(.secondary)
                    }
                    .font(.subheadline)
                }

                if let accountNumber {
                    HStack {
                        Text("Account Number")
                        Spacer()
                        Text("\(accountNumber)")
                            .foregroundColor(.secondary)
                    }
                    .font(.subheadline)
                }

                if let masterFingerprint = manager.rust.masterFingerprint(), !metadata.isTapSigner() {
                    HStack {
                        Text("Fingerprint")
                        Spacer()
                        Text(masterFingerprint)
                            .foregroundColor(.secondary)
                    }
                    .font(.subheadline)
                }

                if case let .tapSigner(t) = metadata.hardwareMetadata {
                    HStack {
                        Text("Card Identifier")
                        Spacer()
                        Text(t.fullCardIdent())
                            .foregroundColor(.secondary)
                            .minimumScaleFactor(0.75)
                    }
                    .font(.subheadline)
                }

                HStack {
                    Text("Wallet Type")
                    Spacer()
                    Text(String(metadata.walletType))
                        .foregroundColor(.secondary)
                }
                .font(.subheadline)
            }

            Section(header: Text("Settings")) {
                HStack {
                    Text("Name")
                    Spacer()

                    Text(metadata.name)
                        .font(.subheadline)
                        .foregroundColor(.secondary)

                    Image(systemName: "chevron.right")
                        .foregroundColor(Color(UIColor.tertiaryLabel))
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
                .contentShape(Rectangle())
                .font(.subheadline)
                .onTapGesture {
                    app.pushRoute(Route.settings(.wallet(id: metadata.id, route: .changeName)))
                }

                VStack(spacing: 14) {
                    HStack {
                        Text("Wallet Color")
                            .font(.subheadline)
                        Spacer()
                    }

                    HStack {
                        Rectangle()
                            .fill(metadata.swiftColor)
                            .cornerRadius(10)
                            .frame(width: 80, height: 80)

                        LazyVGrid(columns: colorColumns, spacing: 20) {
                            ForEach(defaultWalletColors(), id: \.self) { color in
                                ZStack {
                                    if color == metadata.color {
                                        Circle()
                                            .stroke(Color(color).opacity(0.7), lineWidth: 2)
                                            .frame(width: 32, height: 32)
                                    }

                                    Circle()
                                        .fill(Color(color))
                                        .frame(width: 28, height: 28)
                                        .contentShape(Rectangle())
                                }
                                .onTapGesture { manager.dispatch(action: .updateColor(color)) }
                            }
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                        }
                        .frame(maxWidth: .infinity)
                    }
                }
                .padding(.vertical, 8)

                VStack {
                    Toggle(isOn: Binding(
                        get: { manager.walletMetadata.showLabels },
                        set: { _ in manager.dispatch(action: .toggleShowLabels) }
                    )) {
                        Text("Show transaction labels")
                            .font(.subheadline)
                    }
                }
                .padding(.vertical, 1)
            }

            Section(header: Text("Danger Zone")) {
                if manager.walletMetadata.walletType == .hot, manager.hasRecoveryWords() {
                    Button {
                        showingSecretWordsConfirmation = true
                    } label: {
                        Text("View Secret Words")
                            .font(.subheadline)
                    }
                    .confirmationDialog("Are you sure?", isPresented: $showingSecretWordsConfirmation) {
                        Button("Show Me") {
                            app.pushRoute(Route.secretWords(manager.walletMetadata.id))
                        }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text(
                            "Whoever has access to your secret words, has access to your bitcoin. Please keep these safe, don't show them to anyone."
                        )
                    }
                }

                if manager.walletMetadata.walletType == .hot, manager.hasXprvSecret() {
                    Button {
                        showingXprvExportWarning = true
                    } label: {
                        Text("Export Private Key")
                            .font(.subheadline)
                    }
                    .confirmationDialog("Are you sure?", isPresented: $showingXprvExportWarning) {
                        Button("Continue") { showingXprvExportOptions = true }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("Whoever has access to your extended private key, has access to your bitcoin. Please keep it safe, don't show it to anyone.")
                    }
                    .confirmationDialog(
                        "Export Private Key",
                        isPresented: $showingXprvExportOptions,
                        titleVisibility: .visible
                    ) {
                        Button("Share…") { startXprvExport(.share) }
                        Button("Key Teleport") { startXprvExport(.keyTeleport) }
                        Button("Cancel", role: .cancel) {}
                    }
                }

                Button {
                    requiredConfirmations = manager.rust.requiredDeletionConfirmations()
                    showingDeleteConfirmation = true
                } label: {
                    Text("Delete Wallet").foregroundStyle(.red)
                        .font(.subheadline)
                }
                .confirmationDialog("Are you sure?", isPresented: $showingDeleteConfirmation) {
                    Button("Delete", role: .destructive) {
                        if requiredConfirmations >= 2 {
                            showingSecondDeleteConfirmation = true
                        } else {
                            deleteWallet()
                        }
                    }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text(deleteConfirmationMessage)
                }
                .alert("Confirm Deletion", isPresented: $showingSecondDeleteConfirmation) {
                    Button("Delete", role: .destructive) {
                        if requiredConfirmations >= 3 {
                            showingFinalDeleteConfirmation = true
                        } else {
                            deleteWallet()
                        }
                    }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text("Are you sure you want to delete '\(metadata.name)'?")
                }
                .alert("Final Warning", isPresented: $showingFinalDeleteConfirmation) {
                    Button(finalDeleteButtonTitle, role: .destructive) {
                        deleteWallet()
                    }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text(finalDeleteConfirmationMessage)
                }
            }
        }
        .navigationTitle(manager.walletMetadata.name)
        .navigationBarTitleDisplayMode(.inline)
        .foregroundColor(.primary)
        .onDisappear { manager.validateMetadata() }
        .onAppear { manager.validateMetadata() }
        .task {
            accountNumber = manager.rust.nonDefaultAccountNumber()
        }
        .onChange(of: auth.lockState) { _, new in
            guard new == .unlocked, let action = pendingXprvExportAction else { return }
            pendingXprvExportAction = nil
            performXprvExport(action)
        }
        .scrollContentBackground(.hidden)
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
            .environment(AuthManager.shared)
            .environment(\.navigate) { _ in
                ()
            }
            .background(Color(UIColor.systemGroupedBackground))
    }
}
