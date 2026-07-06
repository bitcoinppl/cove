import SwiftUI

extension SendFlowHardwareSheetState: TaggedSheetPresentable {
    func sheet(context: SendFlowHardwarePresentationContext) -> AnyView {
        AnyView(SendFlowHardwareSheetContent(sheet: self, context: context))
    }
}

private struct SendFlowHardwareSheetContent: View {
    let sheet: SendFlowHardwareSheetState
    let context: SendFlowHardwarePresentationContext

    var body: some View {
        switch sheet {
        case .details:
            SendFlowDetailsSheetView(manager: context.manager, details: context.details)
                .presentationDetents([.height(425), .height(600), .large])
                .padding()

        case .inputOutputDetails:
            SendFlowAdvancedDetailsView(manager: context.manager, details: context.details)
                .presentationDetents(
                    [.height(300), .height(400), .height(500), .large],
                    selection: context.inputOutputDetailsPresentationSize
                )

        case .exportQr:
            QrExportView(details: context.details)
                .presentationDetents([.height(550), .height(650), .large])
                .padding()
                .padding(.top, 10)
        }
    }
}
