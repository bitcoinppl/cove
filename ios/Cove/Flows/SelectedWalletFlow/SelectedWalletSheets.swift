import SwiftUI

extension SelectedWalletPresentationState: TaggedSheetPresentable {
    func sheet(context: SelectedWalletPresentationContext) -> AnyView {
        AnyView(SelectedWalletSheetContent(sheet: self, context: context))
    }
}

private struct SelectedWalletSheetContent: View {
    let sheet: SelectedWalletPresentationState
    let context: SelectedWalletPresentationContext

    var body: some View {
        switch sheet {
        case .receive:
            ReceiveView(manager: context.manager)

        case let .chooseAddressType(foundAddresses):
            ChooseWalletTypeView(manager: context.manager, foundAddresses: foundAddresses)

        case .qrLabelsImport:
            QrCodeLabelImportView(scannedCode: context.scannedLabels)

        case .labelsQrExport:
            QrExportView(
                title: "Export Labels",
                subtitle: "Scan to import labels\ninto another wallet",
                generateBbqrStrings: { density in
                    try await context.manager.rust.exportLabelsForQr(density: density)
                },
                generateUrStrings: nil,
                copyData: { try await context.manager.rust.exportLabelsForShare().content }
            )
            .presentationDetents([.height(500), .height(600), .large])
            .padding()
            .padding(.top, 10)

        case .xpubQrExport:
            QrExportView(
                title: "Export Xpub",
                subtitle: "Public descriptor for\nwatch-only wallet",
                generateBbqrStrings: { density in
                    try await context.manager.rust.exportXpubForQr(density: density)
                },
                generateUrStrings: nil,
                copyData: { try await context.manager.rust.exportXpubForShare().content }
            )
            .presentationDetents([.height(500), .height(600), .large])
            .padding()
            .padding(.top, 10)

        case .labelsFileImport, .exportLabelsConfirmation, .exportXpubConfirmation:
            EmptyView()
        }
    }
}
