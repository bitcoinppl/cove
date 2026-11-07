import SwiftUI

/// Lays its views out side by side when each fits on one line, and stacks them otherwise
///
/// At accessibility text sizes, views side by side leave each label too little width, so iOS
/// splits a word across lines and adds a hyphen where it can, as in "Hard-" over "ware". Stacked,
/// each label gets the full width
struct SideBySideOrStacked<Content: View>: View {
    var alignment: HorizontalAlignment = .center
    var spacing: CGFloat?
    @ViewBuilder let content: () -> Content

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: spacing, content: content)
            VStack(alignment: alignment, spacing: spacing, content: content)
        }
    }
}
