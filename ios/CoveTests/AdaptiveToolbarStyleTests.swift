@testable import Cove
import SwiftUI
import XCTest

@MainActor
final class AdaptiveToolbarStyleTests: XCTestCase {
    /// Regression for #927: with Reduce Transparency on, toggling the scroll-driven nav bar
    /// swapped view branches, rebuilding the whole wallet screen and its lazy transaction list
    /// on every toggle, which fed a layout loop during the back transition at large text sizes
    func testNavBarToggleKeepsTheStyledScreenIdentity() throws {
        guard #available(iOS 26.0, *) else {
            throw XCTSkip("the Reduce Transparency styling only exists on iOS 26")
        }

        let appearances = AppearanceCounter()
        let hostingController = UIHostingController(
            rootView: styledScreen(showNavBar: false, appearances: appearances)
        )

        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 393, height: 852))
        window.rootViewController = hostingController
        window.makeKeyAndVisible()
        settle(hostingController)

        XCTAssertEqual(appearances.count, 1, "the screen appears once before any toggle")

        hostingController.rootView = styledScreen(showNavBar: true, appearances: appearances)
        settle(hostingController)
        hostingController.rootView = styledScreen(showNavBar: false, appearances: appearances)
        settle(hostingController)

        XCTAssertEqual(
            appearances.count,
            1,
            "toggling the nav bar rebuilt the screen \(appearances.count - 1) times"
        )
    }

    private func styledScreen(showNavBar: Bool, appearances: AppearanceCounter) -> some View {
        NavigationStack {
            Color.clear
                .onAppear { appearances.count += 1 }
                .adaptiveToolbarStyle(showNavBar: showNavBar, reduceTransparency: true)
        }
    }

    private func settle(_ hostingController: UIViewController) {
        hostingController.view.setNeedsLayout()
        hostingController.view.layoutIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))
    }
}

private final class AppearanceCounter {
    var count = 0
}
