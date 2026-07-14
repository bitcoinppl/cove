import SwiftUI

struct MainSettingsCloudBackupSection: View {
    let isVisible: Bool
    let onEnable: () -> Void
    let onOpenDetail: () -> Void

    @State private var manager = CloudBackupManager.shared

    var body: some View {
        if isVisible {
            Section(header: Text("Cloud Backup")) {
                switch manager.lifecycle {
                case .disabled:
                    SettingsRow(title: "Enable Cloud Backup", symbol: "icloud.and.arrow.up") {
                        onEnable()
                    }
                case .enabling:
                    MainSettingsCloudBackupEnablingRow()
                case .restoring:
                    MainSettingsCloudBackupRestoringRow()
                case .pendingEnableRecovery:
                    cloudBackupRecoveryContent()
                case let .failed(failure):
                    cloudBackupErrorContent(message: failure.message)
                case .configured:
                    MainSettingsCloudBackupEnabledRow(
                        status: manager.settingsRowStatus,
                        onOpenDetail: onOpenDetail
                    )
                }
            }
        }
    }

    @ViewBuilder
    private func cloudBackupRecoveryContent() -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: "exclamationmark.icloud")
                    .foregroundStyle(Color.statusWarning)
                Text("Cloud Backup Needs Recovery")
            }
            Text("Open to review interrupted setup")
                .font(.caption)
                .foregroundStyle(.secondary)
        }

        SettingsRow(title: "Review", symbol: "arrow.right") {
            onEnable()
        }
    }

    @ViewBuilder
    private func cloudBackupErrorContent(message: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: "exclamationmark.icloud")
                    .foregroundStyle(Color.statusError)
                Text("Cloud Backup Error")
            }
            Text(message)
                .font(.caption)
                .foregroundStyle(.secondary)
        }

        SettingsRow(title: "Review", symbol: "arrow.right") {
            onOpenDetail()
        }
    }
}

struct MainSettingsCloudBackupEnablingRow: View {
    var body: some View {
        HStack {
            SettingsIcon(symbol: "icloud.and.arrow.up")
            Text("Setting up cloud backup...")
                .font(.subheadline)
                .padding(8)
            Spacer()
            ProgressView()
        }
    }
}

struct MainSettingsCloudBackupEnabledRow: View {
    let status: CloudBackupSettingsRowStatus
    let onOpenDetail: () -> Void

    var body: some View {
        Button(action: onOpenDetail) {
            HStack {
                MainSettingsCloudBackupEnabledStatus(status: status)
                Spacer()
                settingsChevron
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(minHeight: 44)
        .accessibilityHint("Opens Cloud Backup details")
    }

    private var settingsChevron: some View {
        Image(systemName: "chevron.right")
            .foregroundColor(Color(UIColor.tertiaryLabel))
            .font(.footnote)
            .fontWeight(.semibold)
    }
}

struct MainSettingsCloudBackupEnabledStatus: View {
    let status: CloudBackupSettingsRowStatus

    var body: some View {
        switch status {
        case .disabled, .disabling, .settingUp, .restoring:
            cloudBackupStatusContent(
                symbol: "icloud",
                title: "Cloud Backup",
                color: Color.secondary
            )

        case .passkeyMissing:
            cloudBackupStatusContent(
                symbol: "exclamationmark.icloud.fill",
                title: "Cloud Backup Passkey Missing",
                message: "Backups can't be restored until you add a new passkey",
                color: Color.statusWarning
            )

        case .passkeyProviderUnsupported:
            cloudBackupStatusContent(
                symbol: "exclamationmark.icloud.fill",
                title: "Cloud Backup Passkey Unsupported",
                message: "Open to choose a supported passkey provider",
                color: Color.statusWarning
            )

        case .unverified:
            cloudBackupStatusContent(
                symbol: "exclamationmark.icloud",
                title: "Cloud Backup Unverified",
                color: Color.statusWarning
            )

        case .confirming:
            cloudBackupStatusContent(
                symbol: "arrow.clockwise.icloud",
                title: "Cloud Backup Confirming",
                color: Color.statusInfo
            )

        case .active:
            cloudBackupStatusContent(
                symbol: "checkmark.icloud",
                title: "Cloud Backup Enabled",
                color: Color.statusSuccess
            )

        case .verificationRecommended:
            cloudBackupStatusContent(
                symbol: "exclamationmark.icloud",
                title: "Cloud Backup Enabled",
                message: "Verification recommended",
                color: Color.statusWarning
            )

        case .checkingSync:
            cloudBackupStatusContent(
                symbol: "icloud",
                title: "Checking Cloud Backup",
                message: "Checking iCloud sync status",
                color: Color.secondary
            )

        case .syncing:
            cloudBackupStatusContent(
                symbol: "arrow.clockwise.icloud",
                title: "Cloud Backup Syncing",
                message: "Uploading latest changes",
                color: Color.statusInfo
            )

        case .noFiles:
            cloudBackupStatusContent(
                symbol: "icloud.slash",
                title: "Cloud Backup Needs Attention",
                message: "No iCloud backup files found",
                color: Color.statusWarning
            )

        case .driveUnavailable:
            cloudBackupStatusContent(
                symbol: "exclamationmark.icloud",
                title: "iCloud Drive Unavailable",
                message: "Open to review Cloud Backup",
                color: Color.statusWarning
            )

        case .recoveryRequired:
            cloudBackupStatusContent(
                symbol: "exclamationmark.icloud",
                title: "Cloud Backup Needs Recovery",
                message: "Open to review interrupted setup",
                color: Color.statusWarning
            )

        case let .authorizationRequired(message):
            cloudBackupStatusContent(
                symbol: "exclamationmark.icloud",
                title: "iCloud Access Needed",
                message: message,
                color: Color.statusWarning
            )

        case let .error(message):
            cloudBackupStatusContent(
                symbol: "exclamationmark.icloud",
                title: "Cloud Backup Error",
                message: message,
                color: Color.statusError
            )
        }
    }

    @ViewBuilder
    private func cloudBackupStatusContent(
        symbol: String,
        title: String,
        message: String? = nil,
        color: Color
    ) -> some View {
        Image(systemName: symbol)
            .foregroundStyle(color)

        VStack(alignment: .leading, spacing: 2) {
            Text(title)

            if let message {
                Text(message)
                    .font(.caption2)
                    .foregroundStyle(color)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

struct MainSettingsCloudBackupRestoringRow: View {
    var body: some View {
        HStack {
            ProgressView()
                .padding(.trailing, 8)
            Text("Restoring from cloud backup...")
        }
    }
}
