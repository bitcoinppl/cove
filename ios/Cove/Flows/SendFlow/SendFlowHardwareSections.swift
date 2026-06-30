import SwiftUI

struct SendFlowHardwareAccountSection: View {
    let metadata: WalletMetadata

    var body: some View {
        VStack {
            HStack {
                BitcoinShieldIcon(width: 24, color: .orange)

                VStack(alignment: .leading, spacing: 6) {
                    Text(metadata.identOrFingerprint())
                        .font(.caption)
                        .fontWeight(.medium)
                        .foregroundColor(.secondary)

                    Text(metadata.name)
                        .font(.footnote)
                        .fontWeight(.semibold)
                }
                .padding(.leading, 8)

                Spacer()
            }
        }
    }
}

struct SendFlowHardwareSignTransactionSection: View {
    let exportTransaction: () -> Void
    let importSignature: () -> Void

    var body: some View {
        VStack(spacing: 17) {
            HStack {
                Text("Sign Transaction")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .foregroundColor(.secondary)

                Spacer()
            }

            HStack {
                Button(action: exportTransaction) {
                    SendFlowHardwareActionLabel(
                        title: "Export Transaction",
                        systemImage: "square.and.arrow.up"
                    )
                }

                Spacer()

                Button(action: importSignature) {
                    SendFlowHardwareActionLabel(
                        title: "Import Signature",
                        systemImage: "square.and.arrow.down"
                    )
                }
            }
        }
    }
}

struct SendFlowHardwareExportTransactionDialog: View {
    let exportQr: () -> Void
    let exportNfc: () -> Void
    let shareTransaction: () -> Void

    var body: some View {
        Button("QR Code", action: exportQr)
        Button("NFC", action: exportNfc)
        Button("More...", action: shareTransaction)
    }
}

struct SendFlowHardwareImportTransactionDialog: View {
    let scanQr: () -> Void
    let importFile: () -> Void
    let pasteSignature: () -> Void
    let scanNfc: () -> Void

    var body: some View {
        Button("QR", action: scanQr)
        Button("File", action: importFile)
        Button("Paste", action: pasteSignature)
        Button("NFC", action: scanNfc)
    }
}

private struct SendFlowHardwareActionLabel: View {
    let title: String
    let systemImage: String

    var body: some View {
        Label(title, systemImage: systemImage)
            .padding(.horizontal, 18)
            .padding(.vertical)
            .foregroundColor(.midnightBlue)
            .background(.btnPrimary)
            .cornerRadius(10)
            .font(.caption)
            .fontWeight(.medium)
    }
}
