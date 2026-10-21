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

    func testRootCaptureInjectsTheWindowSafeAreaInsets() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let reader = SafeAreaReader()
        let window = UIWindow(windowScene: scene)
        window.frame = scene.coordinateSpace.bounds
        window.rootViewController = UIHostingController(
            rootView: SafeAreaReaderView(reader: reader).capturesSafeAreaInsets()
        )
        window.isHidden = false
        defer { window.isHidden = true }

        window.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(500))

        XCTAssertGreaterThan(window.safeAreaInsets.top, 0, "the test needs a window with a top inset")
        XCTAssertEqual(reader.insets?.top, window.safeAreaInsets.top)
        XCTAssertEqual(reader.insets?.bottom, window.safeAreaInsets.bottom)
    }
}

@MainActor
@Observable
private final class SafeAreaReader {
    var insets: EdgeInsets?
}

private struct SafeAreaReaderView: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    let reader: SafeAreaReader

    var body: some View {
        Color.clear
            .onChange(of: safeAreaInsets, initial: true) { reader.insets = safeAreaInsets }
    }
}
