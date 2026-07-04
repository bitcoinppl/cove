import SwiftUI

extension HotWalletImportSheetState: TaggedSheetPresentable {
    func sheet(context: HotWalletImportPresentationContext) -> AnyView {
        AnyView(HotWalletImportSheetContent(context: context))
    }
}

private struct HotWalletImportSheetContent: View {
    let context: HotWalletImportPresentationContext

    var body: some View {
        ScannerView(
            codeTypes: [.qr],
            scanMode: .oncePerCode,
            scanInterval: 0.1
        ) { response in
            context.handleScan(response)
        }
        .ignoresSafeArea(.all)
    }
}
