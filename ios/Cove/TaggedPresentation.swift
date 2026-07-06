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

    func presentingFullScreenCover<Item: TaggedSheetPresentable>(
        _ item: Binding<TaggedItem<Item>?>,
        context: Item.SheetPresentationContext
    ) -> some View {
        fullScreenCover(item: item) { taggedItem in
            taggedItem.item.sheet(context: context)
        }
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
