//
//  DynamicScrollView.swift
//  Cove
//
//  Created by Praveen Perera on 6/5/25.
//


import SwiftUI

struct DynamicHeightScrollView<Content: View>: View {
    @Environment(\.sizeCategory) var sizeCategory
    @ViewBuilder
    let content: Content

    var body: some View {
        if isMiniDeviceOrLargeText(sizeCategory) {
            ScrollView { content.frame(idealHeight: screenHeight) }
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
