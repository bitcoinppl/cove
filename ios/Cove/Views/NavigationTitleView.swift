import SwiftUI

/// Sets the navigation bar title via UIKit's navigationItem.titleView,
/// bypassing SwiftUI's ToolbarPlacementEnvironment which has an infinite
/// loop bug with .principal placement during parallax transitions at
/// large accessibility font sizes on iOS 26.
///
/// DO NOT use ToolbarItem(placement: .principal) anywhere in this app.
/// Use this modifier instead. The SwiftUI bug causes 100% CPU freeze
/// when pressing back at accessibilityExtraExtraLarge dynamic type.
/// See: SelectedWalletScreen for usage example
struct NavigationTitleViewModifier<TitleContent: View>: ViewModifier {
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
        vc.clearTitle()
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

    func clearTitle() {
        targetNavigationItem?.titleView = nil
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
