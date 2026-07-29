import SwiftUI

struct BackupImportCompletionMessage: View {
    let message: String?

    var body: some View {
        if let message {
            Text(message)
        }
    }
}

struct BackupImportConfirmationContent: View {
    let report: BackupVerifyReport
    let isImporting: Bool
    let onConfirmImport: () -> Void
    let onBack: () -> Void

    var body: some View {
        VerifyResultView(report: report)

        Section {
            Button(action: onConfirmImport) {
                BackupProgressButtonLabel(
                    title: "Confirm Import",
                    isRunning: isImporting
                )
            }
            .disabled(isImporting)

            Button(action: onBack) {
                HStack {
                    Spacer()
                    Text("Back")
                    Spacer()
                }
            }
        }
    }
}
