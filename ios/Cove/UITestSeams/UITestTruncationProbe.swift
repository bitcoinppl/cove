//
//  UITestTruncationProbe.swift
//  Cove
//
//  Debug-only report of whether a text draws whole, for UI tests
//

import SwiftUI

#if DEBUG
    /// Whether UI tests asked texts to report truncation
    ///
    /// XCUITest reports a text's whole label even when it draws truncated or
    /// clipped, so a test can't see an ellipsis. When
    /// `COVE_UI_TEST_TRUNCATION_PROBE` is `1` in the launch environment, each
    /// probed text sets its accessibility value to `fits` or `truncated`
    enum UITestTruncationProbe {
        static let environmentKey = "COVE_UI_TEST_TRUNCATION_PROBE"

        /// read once, so a probed view keeps one structure for the life of the process
        static let isEnabled = ProcessInfo.processInfo.environment[environmentKey] == "1"

        static let fits = "fits"
        static let truncated = "truncated"

        /// text heights are fractional and can differ in the last bits between two layouts
        static let tolerance: CGFloat = 0.5
    }

    /// Draws the text and a hidden copy with no line limit at the same width,
    /// and reports `truncated` when the copy is taller than the drawn text
    private struct TruncationProbedText: View {
        let text: Text

        @State private var drawnHeight: CGFloat?
        @State private var fullHeight: CGFloat?

        private var value: String {
            guard let drawnHeight, let fullHeight else { return UITestTruncationProbe.truncated }

            return fullHeight - drawnHeight > UITestTruncationProbe.tolerance
                ? UITestTruncationProbe.truncated
                : UITestTruncationProbe.fits
        }

        var body: some View {
            text
                .onGeometryChange(for: CGFloat.self, of: \.size.height) { drawnHeight = $0 }
                .background(alignment: .top) {
                    // the innermost line limit wins, so a limit applied outside the probe can't
                    // cut the copy short too
                    text
                        .lineLimit(nil)
                        .fixedSize(horizontal: false, vertical: true)
                        .hidden()
                        .accessibilityHidden(true)
                        .onGeometryChange(for: CGFloat.self, of: \.size.height) { fullHeight = $0 }
                }
                .accessibilityValue(value)
        }
    }
#endif

extension Text {
    /// Lets UI tests tell whether this text draws whole
    ///
    /// Apply it to the `Text` itself, after modifiers that return `Text` such
    /// as `font` and `fontWeight`, so the copy it measures matches what draws.
    /// Without the debug seam turned on, and in every release build, it returns
    /// the text unchanged
    @ViewBuilder
    func uiTestTruncationProbe() -> some View {
        #if DEBUG
            if UITestTruncationProbe.isEnabled {
                TruncationProbedText(text: self)
            } else {
                self
            }
        #else
            self
        #endif
    }
}
