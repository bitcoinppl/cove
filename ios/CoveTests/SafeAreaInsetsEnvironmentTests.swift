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

    func testShowingTheKeyboardLeavesTheCapturedInsetsUnchanged() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previousKeyWindow = scene.keyWindow
        let reader = SafeAreaReader()
        let window = UIWindow(windowScene: scene)
        window.frame = scene.coordinateSpace.bounds
        window.rootViewController = UIHostingController(
            rootView: KeyboardProbeView(reader: reader).capturesSafeAreaInsets()
        )
        window.makeKeyAndVisible()
        defer {
            window.endEditing(true)
            window.isHidden = true
            previousKeyWindow?.makeKey()
        }

        try await Task.sleep(for: .milliseconds(500))
        let before = try XCTUnwrap(reader.insets)
        let layoutBottomBefore = reader.layoutBottomInset

        reader.isFocused = true
        try await Task.sleep(for: .seconds(1.5))

        // the keyboard-aware layout inset proves the keyboard is up, so the check can't pass vacuously
        XCTAssertGreaterThan(reader.layoutBottomInset, layoutBottomBefore + 100, "the software keyboard must appear")
        XCTAssertEqual(reader.insets, before, "showing the keyboard must not change the injected insets")
    }
}

@MainActor
@Observable
private final class SafeAreaReader {
    var insets: EdgeInsets?
    var layoutBottomInset: CGFloat = 0
    var isFocused = false
}

private struct SafeAreaReaderView: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    let reader: SafeAreaReader

    var body: some View {
        Color.clear
            .onChange(of: safeAreaInsets, initial: true) { reader.insets = safeAreaInsets }
    }
}

/// A text field under the root capture, plus a keyboard-aware reading of the same layout
private struct KeyboardProbeView: View {
    @Environment(\.safeAreaInsets) private var safeAreaInsets
    @FocusState private var isFocused: Bool
    @State private var text = ""
    @Bindable var reader: SafeAreaReader

    var body: some View {
        TextField("Probe", text: $text)
            .focused($isFocused)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .onGeometryChange(for: CGFloat.self, of: \.safeAreaInsets.bottom) { reader.layoutBottomInset = $0 }
            .onChange(of: safeAreaInsets, initial: true) { reader.insets = safeAreaInsets }
            .onChange(of: reader.isFocused) { isFocused = reader.isFocused }
    }
}
