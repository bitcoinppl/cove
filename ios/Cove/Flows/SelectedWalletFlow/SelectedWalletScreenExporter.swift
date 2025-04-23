import SwiftUI
import UniformTypeIdentifiers

private enum AlertState: Equatable {
    case exportSuccess
    case unableToImportLabels(String)
    case unableToExportLabels(String)
}

struct SelctedWalletScreenExporterView: View {
    public enum Exporting {
        case labels
        case backup(ExportingBackup)
        case transactions(String)
    }

    @Environment(AppManager.self) private var app
    let labelManager: LabelManager
    let metadata: WalletMetadata
    @Binding var exporting: Exporting?

    var body: some View {
        switch exporting {
        case .none: EmptyView()
        case .labels:
            EmptyView()
                .fileExporter(
                    isPresented: Binding(
                        get: { exporting != nil },
                        set: { if !$0 { exporting = nil } }
                    ),
                    document: makeJsonLDocument(),
                    contentType: makeContentType(),
                    defaultFilename: makeDefaultFilename(),
                    onCompletion: handle
                )
        default:
            EmptyView()
                .fileExporter(
                    isPresented: Binding(
                        get: { exporting != nil },
                        set: { if !$0 { exporting = nil } }
                    ),
                    document: makeTextDocument(),
                    contentType: makeContentType(),
                    defaultFilename: makeDefaultFilename(),
                    onCompletion: handle
                )
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

    private func makeTextDocument() -> TextDocument {
        switch exporting {
        case let .backup(exportingBackup):
            let data = exportingBackup.backup
            return TextDocument(text: hexEncode(bytes: data))
        case let .transactions(csv):
            return TextDocument(text: csv)
        case .labels:
            fatalError("will never be called when exporting labels")
        case .none:
            fatalError("fileExporter invoked with no export type")
        }
    }

    private func makeJsonLDocument() -> JSONLDocument {
        switch exporting {
        case .labels:
            JSONLDocument(text: exportLabelContent())
        default:
            fatalError("will only be called with exporting is labels")
        }
    }

    // pick UTType
    private func makeContentType() -> UTType {
        switch exporting {
        case .labels:
            .json
        case .backup:
            .plainText
        case .transactions:
            .plainText
        case .none:
            .data
        }
    }

    // pick filename
    private func makeDefaultFilename() -> String {
        switch exporting {
        case .labels:
            return labelManager.exportDefaultFileName(name: metadata.name)
        case let .backup(exportingBackup):
            let prefix = exportingBackup.tapSigner.identFileNamePrefix()
            return "\(prefix)_backup.txt"
        case .transactions:
            return "\(metadata.name)_transactions.csv"
        case .none:
            return "impossible"
        }
    }

    // handle the result based on which export it was
    private func handle(_ result: Result<URL, Error>) {
        switch exporting {
        case .labels:
            switch result {
            case .success:
                app.alertState = .init(
                    .general(title: "Success!", message: "Your labels have been exported!")
                )
            case let .failure(error):
                app.alertState = .init(
                    .general(title: "Ooops something went wrong", message: "Unable to export labels \(error.localizedDescription)")
                )
            }

        case .backup:
            switch result {
            case .success:
                app.sheetState = .none
                app.alertState = .init(
                    .general(
                        title: "Backup Saved!",
                        message: "Your backup has been saved successfully!"
                    )
                )
            case let .failure(error):
                app.alertState = .init(
                    .general(title: "Saving Backup Failed!", message: error.localizedDescription)
                )
            }

        case .transactions:
            app.sheetState = .none
            app.alertState = .init(
                .general(
                    title: "Transactions Exported!",
                    message: "Your transactions have been succesfully exported"
                )
            )

        case .none:
            break
        }

        // reset
        exporting = nil
    }
}
