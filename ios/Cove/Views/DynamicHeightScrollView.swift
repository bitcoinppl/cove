//
//  DynamicHeightScrollView.swift
//  Cove
//
//  Created by Praveen Perera on 3/6/25.
//

import SwiftUI

struct DynamicHeightScrollView<Content: View>: View {
    @State private var contentHeight: CGFloat = 0

    @ViewBuilder
    let content: Content

    var body: some View {
        Group {
            if contentHeight > UIScreen.main.bounds.height {
                ScrollView { content }
            } else {
                VStack { content }
            }
        }
        .background(
            GeometryReader { proxy in
                Color.clear
                    .preference(key: HeightPreferenceKey.self, value: proxy.size.height)
            }
        )
        .onPreferenceChange(HeightPreferenceKey.self) { height in
            contentHeight = height
        }
    }
}

private struct HeightPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

#Preview {
    DynamicHeightScrollView {
        Text("")
    }
}
