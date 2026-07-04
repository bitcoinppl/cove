import SwiftUI

extension AppSheetState: TaggedSheetPresentable {
    func sheet(context: CoveMainPresentationContext) -> AnyView {
        AnyView(AppSheetContent(sheet: self, context: context))
    }
}

private struct AppSheetContent: View {
    let sheet: AppSheetState
    let context: CoveMainPresentationContext

    var body: some View {
        switch sheet {
        case .qr:
            QrCodeScanView(app: context.app, scannedCode: context.scannedCode)

        case let .tapSigner(route):
            TapSignerContainer(route: route)
                .environment(context.app)
        }
    }
}
