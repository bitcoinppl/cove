//
//  ThreeDotsAnimation.swift
//  Cove
//
//  Created by Praveen Perera on 1/12/25.
//

import SwiftUI

struct ThreeDotsAnimation: View {
    @State private var isAnimating = false

    let color: Color = .white
    let size: CGFloat = 6.0
    let number: Int = 3
    let spacing: CGFloat = 8

    var body: some View {
        HStack(spacing: spacing) {
            ForEach(0 ..< number, id: \.self) { index in
                Circle()
                    .fill(color)
                    .frame(width: size, height: size)
                    .scaleEffect(isAnimating ? 1.0 : 0.5)
                    .offset(y: isAnimating ? -size : 0)
                    .animation(
                        .easeInOut(duration: 0.5)
                            .repeatForever()
                            .delay(Double(index) * 0.2),
                        value: isAnimating
                    )
            }
        }
        .onAppear { isAnimating = true }
    }
}

#Preview {
    VStack {
        ThreeDotsAnimation()
    }
    .frame(width: 500, height: 500)
    .background(.black)
}
