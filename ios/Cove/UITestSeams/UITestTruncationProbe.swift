//
//  UITestTruncationProbe.swift
//  Cove
//
//  Debug-only report of whether a text draws whole, for UI tests
//

import SwiftUI
import UIKit

#if DEBUG
    /// Whether UI tests asked texts to report truncation
    ///
    /// XCUITest reports a text's whole label even when it draws truncated or
    /// clipped, so a test can't see an ellipsis. When
    /// `COVE_UI_TEST_TRUNCATION_PROBE` is `1` in the launch environment, each
    /// probed text sets its accessibility value to `fits`, `truncated`,
    /// `hyphenated` when a line break added a hyphen the text doesn't hold, or
    /// `offscreen` when it lies partly past the window's leading or trailing edge
    enum UITestTruncationProbe {
        static let environmentKey = "COVE_UI_TEST_TRUNCATION_PROBE"

        /// read once, so a probed view keeps one structure for the life of the process
        static let isEnabled = ProcessInfo.processInfo.environment[environmentKey] == "1"

        static let fits = "fits"
        static let truncated = "truncated"
        static let hyphenated = "hyphenated"
        static let offscreen = "offscreen"

        /// text heights are fractional and can differ in the last bits between two layouts
        static let tolerance: CGFloat = 0.5

        /// The key window's width, which a text's window frame must stay inside
        @MainActor
        static var windowWidth: CGFloat? {
            UIApplication.shared.connectedScenes
                .compactMap { ($0 as? UIWindowScene)?.keyWindow }
                .first?.bounds.width
        }
    }

    /// Draws the text and a hidden copy with no line limit at the same width,
    /// and reports `truncated` when the copy is taller than the drawn text
    ///
    /// It reports `hyphenated` when the drawn text holds more glyphs than a hidden
    /// copy laid out on one line. At accessibility sizes iOS hyphenates a word that
    /// doesn't fit on a line, and the hyphen it adds is the only glyph a line break
    /// adds, so "Hardware" drawn as "Hard-" and "ware" holds one glyph more. XCUITest
    /// reads the text's string, which never holds that hyphen
    ///
    /// It also reports `offscreen` when the text's frame in the window runs past
    /// either side of it. XCUITest can't see that, since UIKit clips an element's
    /// accessibility frame to its list cell, so a label pushed off the screen
    /// still reads as on it
    private struct TruncationProbedText: View {
        let text: Text

        @State private var drawnFrame: CGRect?
        @State private var drawnHeight: CGFloat?
        @State private var fullHeight: CGFloat?
        @State private var drawnGlyphs: Int?
        @State private var oneLineGlyphs: Int?

        private var value: String {
            guard let drawnFrame, let drawnHeight, let fullHeight, let drawnGlyphs, let oneLineGlyphs else {
                return UITestTruncationProbe.truncated
            }

            let tolerance = UITestTruncationProbe.tolerance
            if fullHeight - drawnHeight > tolerance {
                return UITestTruncationProbe.truncated
            }

            if drawnGlyphs > oneLineGlyphs {
                return UITestTruncationProbe.hyphenated
            }

            let windowWidth = UITestTruncationProbe.windowWidth ?? .infinity
            if drawnFrame.minX < -tolerance || drawnFrame.maxX > windowWidth + tolerance {
                return UITestTruncationProbe.offscreen
            }

            return UITestTruncationProbe.fits
        }

        var body: some View {
            text
                .onPreferenceChange(Text.LayoutKey.self) { drawnGlyphs = $0.glyphCount }
                .onGeometryChange(for: CGRect.self, of: { $0.frame(in: .global) }) { drawnFrame = $0 }
                // the copy's height is in its own space, while a frame in the window shrinks with
                // any scaled container, so the heights compare in the text's own space
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
                .background {
                    text
                        .lineLimit(nil)
                        .fixedSize()
                        .hidden()
                        .accessibilityHidden(true)
                        .onPreferenceChange(Text.LayoutKey.self) { oneLineGlyphs = $0.glyphCount }
                }
                .accessibilityValue(value)
                .preference(
                    key: UITestTruncationReportKey.self,
                    value: UITestTruncationReport(probed: 1, truncated: value == UITestTruncationProbe.fits ? 0 : 1)
                )
        }
    }

    /// How many probed texts a view holds and how many of them draw truncated, hyphenated or
    /// offscreen, or are not yet measured
    struct UITestTruncationReport: Equatable {
        var probed = 0
        var truncated = 0

        /// `fits` only when the view holds a probed text and every one of them draws whole, so a
        /// view whose probes were dropped reads as truncated
        var value: String {
            probed > 0 && truncated == 0 ? UITestTruncationProbe.fits : UITestTruncationProbe.truncated
        }
    }

    private extension [Text.LayoutKey.AnchoredLayout] {
        /// Every glyph the texts lay out, across all their lines
        var glyphCount: Int {
            reduce(0) { total, anchored in
                anchored.layout.reduce(total) { total, line in
                    line.reduce(total) { $0 + $1.count }
                }
            }
        }
    }

    private struct UITestTruncationReportKey: PreferenceKey {
        static let defaultValue = UITestTruncationReport()

        static func reduce(value: inout UITestTruncationReport, nextValue: () -> UITestTruncationReport) {
            let next = nextValue()
            value.probed += next.probed
            value.truncated += next.truncated
        }
    }

    /// An invisible accessibility element behind the view whose value sums up the probed texts
    /// inside it, and whose label counts them, so a test can tell one probe of several was dropped
    ///
    /// It lies behind rather than over the view, since XCUITest finds an element under a tap
    /// point by hit testing, and an element over a row would take the row's taps
    private struct TruncationReportOverlay: ViewModifier {
        let identifier: String

        func body(content: Content) -> some View {
            content.backgroundPreferenceValue(UITestTruncationReportKey.self) { report in
                Color.clear
                    .allowsHitTesting(false)
                    .accessibilityElement()
                    .accessibilityLabel("Truncation report of \(report.probed) texts")
                    .accessibilityIdentifier(identifier)
                    .accessibilityValue(report.value)
            }
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

extension View {
    /// Lets UI tests tell whether every probed text inside this view draws whole, for a view
    /// that merges its texts into one accessibility element, such as a wallet row, so their own
    /// values never reach the UI tree
    ///
    /// With the debug seam turned on, it lays an invisible element with `identifier` behind the
    /// view, whose value is `fits` when the view holds a probed text and all of them draw whole,
    /// and `truncated` otherwise, and whose label is `Truncation report of <n> texts`. Apply it
    /// outside any button that holds the view, since a button merges the elements of its label.
    /// Without the seam, and in every release build, it returns the view unchanged
    @ViewBuilder
    func uiTestTruncationReport(_ identifier: String) -> some View {
        #if DEBUG
            if UITestTruncationProbe.isEnabled {
                modifier(TruncationReportOverlay(identifier: identifier))
            } else {
                self
            }
        #else
            self
        #endif
    }
}

/// A `Label` whose title carries the truncation probe, so a UI test can tell whether it draws
/// whole, from its own value or from that of the button it labels
///
/// At accessibility sizes it shows only its title. Beside the icon, a list row left a word such as
/// "Restore" too little width, so it broke across lines, and above the title the icon made an
/// error too tall to see whole; the title says what the icon shows
struct ProbedLabel: View {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    private let title: Text
    private let systemImage: String

    init(_ title: LocalizedStringKey, systemImage: String) {
        self.title = Text(title)
        self.systemImage = systemImage
    }

    @_disfavoredOverload
    init(_ title: some StringProtocol, systemImage: String) {
        self.title = Text(title)
        self.systemImage = systemImage
    }

    var body: some View {
        let label = Label {
            title.uiTestTruncationProbe()
        } icon: {
            Image(systemName: systemImage)
        }

        if dynamicTypeSize.isAccessibilitySize {
            label.labelStyle(.titleOnly)
        } else {
            label
        }
    }
}
