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
            // the address sits on a card with its own quiet zone, so Receive can stay dark
            ReceiveView(manager: context.manager)
                .qrSheetPresentation(.appAppearance)

        case let .chooseAddressType(foundAddresses):
            ChooseWalletTypeView(manager: context.manager, foundAddresses: foundAddresses)

        case .qrLabelsImport:
            QrCodeLabelImportView(scannedCode: context.scannedLabels)

        case .labelsQrExport:
            LabelQrExportSheet(manager: context.manager)

        case .xpubQrExport:
            XpubQrExportSheet(manager: context.manager)

        case .labelsFileImport, .exportLabelsConfirmation, .exportXpubConfirmation:
            EmptyView()
        }
    }
}

private struct LabelQrExportSheet: View {
    let manager: WalletManager

    var body: some View {
        QrExportView(
            title: "Export Labels",
            subtitle: "Scan to import labels\ninto another wallet",
            generateBbqrStrings: { density in
                try await manager.exportLabelsForQr(density: density)
            },
            generateUr: nil,
            copyData: { try await manager.exportLabelsForShare().content }
        )
        .presentationDetents([.height(500), .height(600), .large])
        .padding()
        .padding(.top, 10)
        .qrSheetPresentation()
    }
}

private struct XpubQrExportSheet: View {
    let manager: WalletManager

    var body: some View {
        QrExportView(
            title: "Export Xpub",
            subtitle: "Public descriptor for\nwatch-only wallet",
            generateBbqrStrings: { density in
                try await manager.exportXpubForQr(density: density)
            },
            generateUr: nil,
            copyData: { try await manager.exportXpubForShare().content }
        )
        .presentationDetents([.height(500), .height(600), .large])
        .padding()
        .padding(.top, 10)
        .qrSheetPresentation()
    }
}
