import SwiftUI

struct VerifyResultView: View {
    let report: BackupVerifyReport

    var body: some View {
        BackupVerifiedSuccessSection()
        BackupInfoSection(
            createdAt: report.createdAt,
            walletCount: report.walletCount
        )

        ForEach(Array(report.wallets.enumerated()), id: \.offset) { _, wallet in
            BackupVerifyWalletSection(wallet: wallet)
        }

        BackupVerifySettingsSection(
            fiatCurrency: report.fiatCurrency,
            colorScheme: report.colorScheme,
            nodeConfigCount: report.nodeConfigCount
        )
    }
}

private struct BackupVerifiedSuccessSection: View {
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
    }
}

private struct BackupInfoSection: View {
    let createdAt: UInt64
    let walletCount: UInt32

    var body: some View {
        Section("Backup Info") {
            LabeledContent("Created", value: formatDate(createdAt))
            LabeledContent("Wallets", value: "\(walletCount)")
        }
    }

    private func formatDate(_ timestamp: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(timestamp))

        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

private struct BackupVerifyWalletSection: View {
    let wallet: BackupWalletSummary

    var body: some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                BackupVerifyWalletHeader(
                    name: wallet.name,
                    alreadyOnDevice: wallet.alreadyOnDevice
                )

                Divider()

                BackupVerifyWalletMetadata(wallet: wallet)

                if let warning = wallet.warning {
                    Label(warning, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundColor(.orange)
                }
            }
            .padding(.vertical, 4)
        }
    }
}

private struct BackupVerifyWalletHeader: View {
    let name: String
    let alreadyOnDevice: Bool

    var body: some View {
        HStack {
            Text(name)
                .fontWeight(.medium)
            Spacer()
            Text(alreadyOnDevice ? "Already on device" : "New")
                .font(.caption)
                .fontWeight(.medium)
                .foregroundColor(alreadyOnDevice ? .secondary : .green)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(
                    alreadyOnDevice
                        ? Color.secondary.opacity(0.15)
                        : Color.green.opacity(0.15),
                    in: Capsule()
                )
        }
    }
}

private struct BackupVerifyWalletMetadata: View {
    let wallet: BackupWalletSummary

    var body: some View {
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
    }
}

private struct BackupVerifySettingsSection: View {
    let fiatCurrency: String?
    let colorScheme: String?
    let nodeConfigCount: UInt32

    var body: some View {
        Section("Settings") {
            if let fiat = fiatCurrency {
                LabeledContent("Fiat Currency", value: fiat)
            }

            if let scheme = colorScheme {
                LabeledContent("Color Scheme", value: scheme)
            }

            LabeledContent("Node Configs", value: "\(nodeConfigCount)")
        }
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
