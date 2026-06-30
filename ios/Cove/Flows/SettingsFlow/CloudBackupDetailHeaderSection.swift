import SwiftUI

struct HeaderSection: View {
    let lastSync: UInt64?
    let syncHealth: CloudSyncHealth

    var body: some View {
        Section {
            VStack(spacing: 8) {
                CloudBackupHeaderIcon(syncHealth: syncHealth)
                    .font(.largeTitle)

                Text(cloudBackupDetailHeaderTitle(syncHealth: syncHealth))
                    .fontWeight(.semibold)

                if let lastSync {
                    Text("Last synced \(formatDate(lastSync))")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                CloudBackupSyncHealthLabel(syncHealth: syncHealth)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
        }
    }

    private func formatDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

private struct CloudBackupHeaderIcon: View {
    let syncHealth: CloudSyncHealth

    var body: some View {
        let image = Image(systemName: cloudBackupDetailHeaderIconName(syncHealth: syncHealth))

        switch syncHealth {
        case .unknown:
            image.foregroundColor(.secondary)
        case .allUploaded:
            image
                .foregroundColor(.statusSuccess)
        case .uploading:
            image
                .foregroundColor(.statusInfo)
        case .failed:
            image
                .foregroundColor(.statusError)
        case .authorizationRequired, .noFiles, .unavailable:
            image
                .foregroundColor(.statusWarning)
        }
    }
}

private struct CloudBackupSyncHealthLabel: View {
    let syncHealth: CloudSyncHealth

    var body: some View {
        switch syncHealth {
        case .unknown:
            CloudBackupSyncProgressLabel(title: "Checking iCloud sync status...")
        case .allUploaded:
            Label("All files synced to iCloud", systemImage: "checkmark.circle.fill")
                .font(.caption)
                .foregroundStyle(Color.statusSuccess)
        case .uploading:
            CloudBackupSyncProgressLabel(title: "Syncing to iCloud...")
        case let .failed(message):
            Label("Sync error: \(message)", systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(Color.statusError)
        case .authorizationRequired:
            Label("iCloud Drive access needs to be reconnected", systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(Color.statusWarning)
        case .noFiles:
            Label("No iCloud backup files uploaded yet", systemImage: "icloud.slash")
                .font(.caption)
                .foregroundStyle(Color.statusWarning)
        case .unavailable:
            Label("iCloud Drive is unavailable", systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(Color.statusWarning)
        }
    }
}

private struct CloudBackupSyncProgressLabel: View {
    let title: String

    var body: some View {
        HStack(spacing: 4) {
            ProgressView()
                .controlSize(.mini)
            Text(title)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}

private func cloudBackupDetailHeaderTitle(syncHealth: CloudSyncHealth) -> String {
    switch syncHealth {
    case .allUploaded:
        "Cloud Backup Active"
    case .uploading:
        "Cloud Backup Syncing"
    case .unknown:
        "Checking Cloud Backup"
    case .noFiles, .authorizationRequired, .unavailable, .failed:
        "Cloud Backup Needs Attention"
    }
}

private func cloudBackupDetailHeaderIconName(syncHealth: CloudSyncHealth) -> String {
    switch syncHealth {
    case .allUploaded:
        "checkmark.icloud.fill"
    case .uploading:
        "arrow.clockwise.icloud.fill"
    case .unknown:
        "icloud"
    case .noFiles:
        "icloud.slash"
    case .authorizationRequired, .unavailable, .failed:
        "exclamationmark.icloud.fill"
    }
}
