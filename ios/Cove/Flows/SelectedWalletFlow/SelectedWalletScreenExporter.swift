import SwiftUI
import UniformTypeIdentifiers

struct SelctedWalletScreenExporterView: View {
    public enum Exporting: Equatable {
        case labels
        case backup(ExportingBackup)
        case transactions(String)
    }

    @Environment(AppManager.self) private var app
    let labelManager: LabelManager
    let metadata: WalletMetadata
    @Binding var exporting: Exporting?

    var body: some View {
        VStack {}
            .onChange(of: exporting) { _, newValue in
                guard let exportType = newValue else { return }
                handleExport(exportType)
            }
    }

    private func handleExport(_ exportType: Exporting) {
        let (content, filename) = makeExportData(exportType)

        ShareSheetHandler.presentShareSheet(
            data: content,
            filename: filename,
            utType: .plainText
        ) { success in
            handleCompletion(exportType: exportType, success: success)
        }
    }

    private func makeExportData(_ exportType: Exporting) -> (content: String, filename: String) {
        switch exportType {
        case .labels:
            let content = exportLabelContent()
            let filename = labelManager.exportDefaultFileName(name: metadata.name)
            return (content, filename)

        case let .backup(exportingBackup):
            let content = hexEncode(bytes: exportingBackup.backup)
            let prefix = exportingBackup.tapSigner.identFileNamePrefix()
            let filename = "\(prefix)_backup.txt"
            return (content, filename)

        case let .transactions(csv):
            let filename = "\(metadata.name.lowercased())_transactions.csv"
            return (csv, filename)
        }
    }

    private func exportLabelContent() -> String {
        do {
            return try labelManager.export()
        } catch {
            app.alertState = .init(
                .general(
                    title: "Oops something went wrong!",
                    message: "Error exporting labels \(error.localizedDescription)"
                )
            )
            return ""
        }
    }

    private func handleCompletion(exportType: Exporting, success: Bool) {
        // reset exporting state
        exporting = nil

        // only show alerts on success, not on cancellation
        guard success else { return }

        switch exportType {
        case .labels:
            app.alertState = .init(
                .general(title: "Success!", message: "Your labels have been exported!")
            )

        case .backup:
            app.sheetState = .none
            app.alertState = .init(
                .general(
                    title: "Backup Saved!",
                    message: "Your backup has been saved successfully!"
                )
            )

        case .transactions:
            app.sheetState = .none
            app.alertState = .init(
                .general(
                    title: "Transactions Exported!",
                    message: "Your transactions have been succesfully exported"
                )
            )
        }
    }
}
