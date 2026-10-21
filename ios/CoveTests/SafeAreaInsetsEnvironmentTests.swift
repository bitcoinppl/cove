@testable import Cove
import SwiftUI
import XCTest

/// Regression for #941: reading `\.safeAreaInsets` asked the key window for its insets while
/// SwiftUI updated its graph, which re-entered the graph through the status bar and froze the app
@MainActor
final class SafeAreaInsetsEnvironmentTests: XCTestCase {
    func testDefaultSafeAreaInsetsDoNotReadTheKeyWindow() throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let keyWindow = try XCTUnwrap(scene.keyWindow)
        XCTAssertGreaterThan(keyWindow.safeAreaInsets.top, 0, "the test needs a key window with a top inset")

        XCTAssertEqual(
            EnvironmentValues().safeAreaInsets,
            EdgeInsets(),
            "the default reads the key window's insets, a UIKit layout read inside SwiftUI graph updates"
        )
    }
}
