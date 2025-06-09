//
//  DynamicScrollView.swift
//  Cove
//
//  Created by Praveen Perera on 6/5/25.
//

import SwiftUI

struct DynamicHeightScrollView<Content: View>: View {
    @Environment(\.sizeCategory) var sizeCategory

    var idealHeight: CGFloat? = screenHeight
    var scrollIndicators: ScrollIndicatorVisibility = .hidden

    @ViewBuilder
    let content: Content

    var body: some View {
        if isMiniDeviceOrLargeText(sizeCategory) {
            ScrollView {
                if let idealHeight {
                    content.frame(idealHeight: idealHeight)
                } else {
                    content
                }
            }
            .scrollIndicators(scrollIndicators)
        } else {
            content
        }
    }
}

#Preview {
    DynamicHeightScrollView {
        Text("")
    }
}
