import SwiftUI

struct SendFlowHardwareDialogActions {
    let exportQr: () -> Void
    let exportNfc: () -> Void
    let shareTransaction: () -> Void
    let scanQr: () -> Void
    let importFile: () -> Void
    let pasteSignature: () -> Void
    let scanNfc: () -> Void
}

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
    let exportDialogIsPresented: Binding<Bool>
    let importDialogIsPresented: Binding<Bool>
    let dialogActions: SendFlowHardwareDialogActions

    var body: some View {
        VStack(spacing: 17) {
            HStack {
                Text("Sign Transaction")
                    .font(.footnote)
                    .fontWeight(.medium)
                    .uiTestTruncationProbe()
                    .foregroundColor(.secondary)

                Spacer()
            }

            // side by side at the largest sizes, each label had room for only part of a word and
            // iOS split it with an added hyphen, so the buttons stack when both can't fit on a line
            ViewThatFits(in: .horizontal) {
                HStack {
                    SendFlowHardwareExportButton(
                        exportTransaction: exportTransaction,
                        isPresented: exportDialogIsPresented,
                        dialogActions: dialogActions
                    )
                    Spacer()
                    SendFlowHardwareImportButton(
                        importSignature: importSignature,
                        isPresented: importDialogIsPresented,
                        dialogActions: dialogActions
                    )
                }

                VStack(alignment: .leading) {
                    SendFlowHardwareExportButton(
                        exportTransaction: exportTransaction,
                        isPresented: exportDialogIsPresented,
                        dialogActions: dialogActions
                    )
                    SendFlowHardwareImportButton(
                        importSignature: importSignature,
                        isPresented: importDialogIsPresented,
                        dialogActions: dialogActions
                    )
                }
            }
        }
    }
}

private struct SendFlowHardwareExportButton: View {
    let exportTransaction: () -> Void
    let isPresented: Binding<Bool>
    let dialogActions: SendFlowHardwareDialogActions

    var body: some View {
        Button(action: exportTransaction) {
            SendFlowHardwareActionLabel(title: "Export Transaction", systemImage: "square.and.arrow.up")
        }
        .confirmationDialog("Export Transaction", isPresented: isPresented) {
            SendFlowHardwareExportTransactionDialog(
                exportQr: dialogActions.exportQr,
                exportNfc: dialogActions.exportNfc,
                shareTransaction: dialogActions.shareTransaction
            )
        }
    }
}

private struct SendFlowHardwareImportButton: View {
    let importSignature: () -> Void
    let isPresented: Binding<Bool>
    let dialogActions: SendFlowHardwareDialogActions

    var body: some View {
        Button(action: importSignature) {
            SendFlowHardwareActionLabel(title: "Import Signature", systemImage: "square.and.arrow.down")
        }
        .confirmationDialog("Import Signature", isPresented: isPresented) {
            SendFlowHardwareImportTransactionDialog(
                scanQr: dialogActions.scanQr,
                importFile: dialogActions.importFile,
                pasteSignature: dialogActions.pasteSignature,
                scanNfc: dialogActions.scanNfc
            )
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
        Label {
            Text(title).uiTestTruncationProbe()
        } icon: {
            Image(systemName: systemImage)
        }
        .padding(.horizontal, 18)
        .padding(.vertical)
        .foregroundColor(.midnightBlue)
        .background(.btnPrimary)
        .cornerRadius(10)
        .font(.caption)
        .fontWeight(.medium)
    }
}
