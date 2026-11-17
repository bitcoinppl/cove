//
//  FillHeightLayout.swift
//  Cove
//

import SwiftUI

/// Places its content at least `minHeight` tall, so spacers inside it can grow
///
/// A scroll view proposes no height, so `.frame(minHeight:)` there centers content at its ideal
/// height instead of stretching it; this layout gives the content a concrete height to fill
struct FillHeightLayout: Layout {
    let minHeight: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache _: inout ()) -> CGSize {
        let ideal = idealSize(of: subviews, width: proposal.width)
        return CGSize(width: proposal.width ?? ideal.width, height: max(ideal.height, minHeight))
    }

    func placeSubviews(in bounds: CGRect, proposal _: ProposedViewSize, subviews: Subviews, cache _: inout ()) {
        for subview in subviews {
            subview.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
        }
    }

    private func idealSize(of subviews: Subviews, width: CGFloat?) -> CGSize {
        subviews
            .map { $0.sizeThatFits(ProposedViewSize(width: width, height: nil)) }
            .reduce(CGSize.zero) { CGSize(width: max($0.width, $1.width), height: max($0.height, $1.height)) }
    }
}
