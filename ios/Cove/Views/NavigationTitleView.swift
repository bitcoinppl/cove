import SwiftUI

// MARK: - iOS 26 Workaround: UIKit Navigation Title

//
// SwiftUI has a bug on iOS 26 where ToolbarPlacementEnvironment.updateValue()
// enters an infinite loop during _UINavigationParallaxTransition when a
// .principal toolbar item exists at large accessibility font sizes
// (specifically accessibilityExtraExtraLarge). This causes 100% CPU freeze
// when pressing the back button.
//
// This file provides .navigationTitleView { } as a drop-in replacement for
// ToolbarItem(placement: .principal). It hosts SwiftUI content inside a
// UIHostingController assigned to UIKit's navigationItem.titleView, which
// bypasses SwiftUI's toolbar placement system entirely while keeping the
// centered title appearance.
//
// Usage:
//   .navigationTitleView {
//       Text("My Title")
//           .font(.callout)
//           .fontWeight(.semibold)
//           .foregroundStyle(.white)
//   }
//
// DO NOT use ToolbarItem(placement: .principal) anywhere in this app

extension View {
    /// Sets a centered navigation bar title via UIKit, bypassing the SwiftUI
    /// .principal toolbar placement bug on iOS 26
    func navigationTitleView(
        @ViewBuilder _ content: @escaping () -> some View
    ) -> some View {
        modifier(NavigationTitleViewModifier(titleContent: content))
    }
}

private struct NavigationTitleViewModifier<TitleContent: View>: ViewModifier {
    @ViewBuilder let titleContent: () -> TitleContent

    func body(content: Content) -> some View {
        content.background(
            NavigationTitleHelper(titleContent: titleContent)
                .frame(width: 0, height: 0)
                .accessibilityHidden(true)
        )
    }
}

private struct NavigationTitleHelper<TitleContent: View>: UIViewControllerRepresentable {
    @ViewBuilder let titleContent: () -> TitleContent

    func makeUIViewController(context _: Context) -> NavigationTitleViewController<TitleContent> {
        NavigationTitleViewController(titleContent: titleContent())
    }

    func updateUIViewController(_ vc: NavigationTitleViewController<TitleContent>, context _: Context) {
        vc.updateTitle(titleContent())
    }

    static func dismantleUIViewController(
        _ vc: NavigationTitleViewController<TitleContent>, coordinator _: Coordinator
    ) {
        vc.clearOwnedTitle()
    }

    typealias UIViewControllerType = NavigationTitleViewController<TitleContent>
}

final class NavigationTitleViewController<TitleContent: View>: UIViewController {
    private var hostingController: UIHostingController<TitleContent>?
    private weak var targetNavigationItem: UINavigationItem?

    init(titleContent: TitleContent) {
        super.init(nibName: nil, bundle: nil)

        let hosting = UIHostingController(rootView: titleContent)
        hosting.view.backgroundColor = .clear
        hosting.sizingOptions = .intrinsicContentSize
        hosting.view.translatesAutoresizingMaskIntoConstraints = false
        hostingController = hosting
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func didMove(toParent parent: UIViewController?) {
        super.didMove(toParent: parent)
        if parent != nil {
            attachTitleView()
        }
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        attachTitleView()
    }

    func updateTitle(_ content: TitleContent) {
        hostingController?.rootView = content
        hostingController?.view.invalidateIntrinsicContentSize()
        hostingController?.view.sizeToFit()
    }

    func clearOwnedTitle() {
        guard let navigationItem = targetNavigationItem else { return }

        if navigationItem.titleView === hostingController?.view {
            navigationItem.titleView = nil
        }

        targetNavigationItem = nil
    }

    private func attachTitleView() {
        guard let hostingView = hostingController?.view else { return }
        guard let navItem = findNavigationItem() else { return }

        if navItem.titleView !== hostingView {
            navItem.titleView = hostingView
        }

        targetNavigationItem = navItem
    }

    /// Finds the nearest ancestor VC that participates in the
    /// navigation controller's viewControllers array
    private func findNavigationItem() -> UINavigationItem? {
        guard let navController = navigationController else { return nil }
        let navViewControllers = Set(navController.viewControllers.map(ObjectIdentifier.init))

        var current: UIViewController? = parent
        while let vc = current {
            if navViewControllers.contains(ObjectIdentifier(vc)) {
                return vc.navigationItem
            }
            current = vc.parent
        }

        return navController.topViewController?.navigationItem
    }
}
