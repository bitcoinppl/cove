import SwiftUI

struct MainSettingsCloudBackupSection: View {
    let isVisible: Bool
    let onEnable: () -> Void
    let onOpenDetail: () -> Void

    @State private var manager = CloudBackupManager.shared

    var body: some View {
        if isVisible {
            Section(header: Text("Cloud Backup")) {
                MainSettingsCloudBackupLifecycleContent(
                    lifecycle: manager.lifecycle,
                    status: manager.settingsRowStatus,
                    onEnable: onEnable,
                    onOpenDetail: onOpenDetail
                )
            }
        }
    }
}

private struct MainSettingsCloudBackupLifecycleContent: View {
    private let content: AnyView

    init(
        lifecycle: CloudBackupLifecycle,
        status: CloudBackupSettingsRowStatus,
        onEnable: @escaping () -> Void,
        onOpenDetail: @escaping () -> Void
    ) {
        content = switch lifecycle {
        case .disabled:
            AnyView(SettingsRow(
                title: "Enable Cloud Backup",
                symbol: "icloud.and.arrow.up",
                onTapGesture: onEnable
            ))
        case .enabling:
            AnyView(MainSettingsCloudBackupEnablingRow())
        case .restoring:
            AnyView(MainSettingsCloudBackupRestoringRow())
        case .pendingEnableRecovery:
            AnyView(MainSettingsCloudBackupRecoveryContent(onReview: onEnable))
        case let .failed(failure):
            AnyView(MainSettingsCloudBackupErrorContent(
                message: failure.message,
                onReview: onOpenDetail
            ))
        case .configured:
            AnyView(MainSettingsCloudBackupEnabledRow(
                status: status,
                onOpenDetail: onOpenDetail
            ))
        }
    }

    var body: some View {
        content
    }
}

private struct MainSettingsCloudBackupRecoveryContent: View {
    let onReview: () -> Void

    var body: some View {
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

        SettingsRow(title: "Review", symbol: "arrow.right", onTapGesture: onReview)
    }
}

private struct MainSettingsCloudBackupErrorContent: View {
    let message: String
    let onReview: () -> Void

    var body: some View {
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

        SettingsRow(title: "Review", symbol: "arrow.right", onTapGesture: onReview)
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
    private let presentation: MainSettingsCloudBackupStatusPresentation

    init(status: CloudBackupSettingsRowStatus) {
        presentation = MainSettingsCloudBackupStatusPresentation(status: status)
    }

    var body: some View {
        Image(systemName: presentation.symbol)
            .foregroundStyle(presentation.color)

        VStack(alignment: .leading, spacing: 2) {
            Text(presentation.title)

            if let message = presentation.message {
                Text(message)
                    .font(.caption2)
                    .foregroundStyle(presentation.color)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

private struct MainSettingsCloudBackupStatusPresentation {
    let symbol: String
    let title: String
    let message: String?
    let color: Color

    private init(symbol: String, title: String, message: String?, color: Color) {
        self.symbol = symbol
        self.title = title
        self.message = message
        self.color = color
    }

    init(status: CloudBackupSettingsRowStatus) {
        self = switch status {
        case .disabled, .disabling, .settingUp, .restoring:
            .inactive
        case .passkeyMissing:
            .passkeyMissing
        case .passkeyProviderUnsupported:
            .passkeyProviderUnsupported
        case .unverified:
            .unverified
        case .confirming:
            .confirming
        case .active:
            .active
        case .verificationRecommended:
            .verificationRecommended
        case .checkingSync:
            .checkingSync
        case .syncing:
            .syncing
        case .noFiles:
            .noFiles
        case .driveUnavailable:
            .driveUnavailable
        case .recoveryRequired:
            .recoveryRequired
        case let .authorizationRequired(message):
            .init(
                symbol: "exclamationmark.icloud",
                title: "iCloud Access Needed",
                message: message,
                color: Color.statusWarning
            )
        case let .error(message):
            .init(
                symbol: "exclamationmark.icloud",
                title: "Cloud Backup Error",
                message: message,
                color: Color.statusError
            )
        }
    }

    private static let inactive = Self(
        symbol: "icloud",
        title: "Cloud Backup",
        message: nil,
        color: Color.secondary
    )
    private static let passkeyMissing = Self(
        symbol: "exclamationmark.icloud.fill",
        title: "Cloud Backup Passkey Missing",
        message: "Backups can't be restored until you add a new passkey",
        color: Color.statusWarning
    )
    private static let passkeyProviderUnsupported = Self(
        symbol: "exclamationmark.icloud.fill",
        title: "Cloud Backup Passkey Unsupported",
        message: "Open to choose a supported passkey provider",
        color: Color.statusWarning
    )
    private static let unverified = Self(
        symbol: "exclamationmark.icloud",
        title: "Cloud Backup Unverified",
        message: nil,
        color: Color.statusWarning
    )
    private static let confirming = Self(
        symbol: "arrow.clockwise.icloud",
        title: "Cloud Backup Confirming",
        message: nil,
        color: Color.statusInfo
    )
    private static let active = Self(
        symbol: "checkmark.icloud",
        title: "Cloud Backup Enabled",
        message: nil,
        color: Color.statusSuccess
    )
    private static let verificationRecommended = Self(
        symbol: "exclamationmark.icloud",
        title: "Cloud Backup Enabled",
        message: "Verification recommended",
        color: Color.statusWarning
    )
    private static let checkingSync = Self(
        symbol: "icloud",
        title: "Checking Cloud Backup",
        message: "Checking iCloud sync status",
        color: Color.secondary
    )
    private static let syncing = Self(
        symbol: "arrow.clockwise.icloud",
        title: "Cloud Backup Syncing",
        message: "Uploading latest changes",
        color: Color.statusInfo
    )
    private static let noFiles = Self(
        symbol: "icloud.slash",
        title: "Cloud Backup Needs Attention",
        message: "No iCloud backup files found",
        color: Color.statusWarning
    )
    private static let driveUnavailable = Self(
        symbol: "exclamationmark.icloud",
        title: "iCloud Drive Unavailable",
        message: "Open to review Cloud Backup",
        color: Color.statusWarning
    )
    private static let recoveryRequired = Self(
        symbol: "exclamationmark.icloud",
        title: "Cloud Backup Needs Recovery",
        message: "Open to review interrupted setup",
        color: Color.statusWarning
    )
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
