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

    /// Presents a sheet and an alert from one view, holding the alert back while any presentation
    /// in the window is still sliding in or away
    ///
    /// UIKit can't present an alert from a view that is still dismissing a sheet, and SwiftUI then
    /// drops the alert for good. That happens when one update closes a sheet and raises the alert,
    /// such as a QR code that scans while the scanner sheet is still sliding up, or a Receive sheet
    /// that closes because its address failed to load and reports it through the app's alert. The
    /// sheet can be this view's own or one a screen below it presents, which this view can't see,
    /// so the alert waits until UIKit has no presentation in flight. An alert raised while a sheet
    /// stays open shows over it
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

    /// The alert cleared to show, once no presentation in the window was sliding in or away
    @State private var releasedAlertID: UUID?

    private var alertShown: Binding<Bool> {
        Binding(
            get: { alert != nil && alert?.id == releasedAlertID },
            set: { isPresented in
                // only the alert on screen can be dismissed, not a newer one still held back
                if !isPresented, alert?.id == releasedAlertID { alert = nil }
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
        .task(id: alert?.id) { await releaseAlertOncePresenterIsIdle() }
    }

    @ViewBuilder
    private func presentingSheet(_ content: some View) -> some View {
        switch style {
        case .sheet:
            content.sheet(item: $sheet, content: sheetContent)
        case .fullScreenCover:
            content.fullScreenCover(item: $sheet, content: sheetContent)
        }
    }

    private func sheetContent(_ taggedItem: TaggedItem<SheetItem>) -> some View {
        taggedItem.item.sheet(context: sheetContext)
    }

    private func releaseAlertOncePresenterIsIdle() async {
        guard let alertID = alert?.id else { return }

        // a dismissal the same update started can reach UIKit just after this task starts, and
        // the presenter counts as idle only on consecutive checks, so the wait still sees it.
        // A wait that times out starts again, since releasing the alert over a presentation still
        // in flight drops it. The app lock is a SwiftUI overlay that doesn't stop an alert from
        // presenting, so unlike a passkey request the alert waits for an idle window even then
        while !Task.isCancelled {
            if case .ready = await PresenterIdle.waitUntilReady(isAppLocked: { false }) { break }
        }

        guard !Task.isCancelled else { return }

        releasedAlertID = alertID
    }
}
