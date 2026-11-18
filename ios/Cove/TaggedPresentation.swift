import SwiftUI

protocol TaggedAlertPresentable {
    associatedtype AlertPresentationContext

    func alert(context: AlertPresentationContext) -> AnyAlertBuilder
}

protocol TaggedSheetPresentable {
    associatedtype SheetPresentationContext

    func sheet(context: SheetPresentationContext) -> AnyView
}

extension View {
    func presentingAlert<Item: TaggedAlertPresentable>(
        _ item: Binding<TaggedItem<Item>?>,
        context: Item.AlertPresentationContext,
        defaultTitle: String = "Alert"
    ) -> some View {
        alert(
            item.wrappedValue?.item.alert(context: context).title ?? defaultTitle,
            isPresented: isPresenting(item),
            presenting: item.wrappedValue,
            actions: { $0.item.alert(context: context).actions },
            message: { $0.item.alert(context: context).message }
        )
    }

    func presentingSheet<Item: TaggedSheetPresentable>(
        _ item: Binding<TaggedItem<Item>?>,
        context: Item.SheetPresentationContext
    ) -> some View {
        sheet(item: item) { taggedItem in
            taggedItem.item.sheet(context: context)
        }
    }

    /// Presents a sheet and an alert from one view, holding the alert back while the sheet is
    /// sliding away
    ///
    /// UIKit can't present an alert from a view that is still dismissing a sheet, and SwiftUI then
    /// drops the alert for good. That happens when one update closes the sheet and raises the
    /// alert, such as a QR code that scans while the scanner sheet is still sliding up, so the
    /// alert waits for the sheet's dismissal to finish. An alert raised while the sheet stays open
    /// shows over it at once
    func presentingSheetAndAlert<SheetItem: TaggedSheetPresentable, AlertItem: TaggedAlertPresentable>(
        sheet: Binding<TaggedItem<SheetItem>?>,
        sheetContext: SheetItem.SheetPresentationContext,
        style: SheetPresentationStyle = .sheet,
        alert: Binding<TaggedItem<AlertItem>?>,
        alertContext: AlertItem.AlertPresentationContext,
        defaultTitle: String = "Alert"
    ) -> some View {
        modifier(
            SheetAndAlertPresentation(
                sheet: sheet,
                sheetContext: sheetContext,
                style: style,
                alert: alert,
                alertContext: alertContext,
                defaultTitle: defaultTitle
            )
        )
    }

    private func isPresenting(_ item: Binding<TaggedItem<some Any>?>) -> Binding<Bool> {
        Binding(
            get: { item.wrappedValue != nil },
            set: { isPresented in
                if !isPresented { item.wrappedValue = nil }
            }
        )
    }
}

/// How `presentingSheetAndAlert` shows its sheet
enum SheetPresentationStyle {
    case sheet
    case fullScreenCover
}

private struct SheetAndAlertPresentation<SheetItem: TaggedSheetPresentable, AlertItem: TaggedAlertPresentable>: ViewModifier {
    @Binding var sheet: TaggedItem<SheetItem>?
    let sheetContext: SheetItem.SheetPresentationContext
    let style: SheetPresentationStyle
    @Binding var alert: TaggedItem<AlertItem>?
    let alertContext: AlertItem.AlertPresentationContext
    let defaultTitle: String

    /// From the sheet's content appearing until its dismissal finishes, which outlasts the
    /// binding going nil by the length of the slide down
    @State private var sheetOnScreen = false

    /// The sheet was closed but is still sliding down
    private var sheetDismissing: Bool {
        sheet == nil && sheetOnScreen
    }

    private var alertShown: Binding<Bool> {
        Binding(
            get: { alert != nil && !sheetDismissing },
            set: { isPresented in
                if !isPresented { alert = nil }
            }
        )
    }

    func body(content: Content) -> some View {
        presentingSheet(
            content.alert(
                alert?.item.alert(context: alertContext).title ?? defaultTitle,
                isPresented: alertShown,
                presenting: alert,
                actions: { $0.item.alert(context: alertContext).actions },
                message: { $0.item.alert(context: alertContext).message }
            )
        )
    }

    @ViewBuilder
    private func presentingSheet(_ content: some View) -> some View {
        switch style {
        case .sheet:
            content.sheet(item: $sheet, onDismiss: { sheetOnScreen = false }, content: sheetContent)
        case .fullScreenCover:
            content.fullScreenCover(item: $sheet, onDismiss: { sheetOnScreen = false }, content: sheetContent)
        }
    }

    private func sheetContent(_ taggedItem: TaggedItem<SheetItem>) -> some View {
        taggedItem.item.sheet(context: sheetContext)
            .onAppear { sheetOnScreen = true }
    }
}
