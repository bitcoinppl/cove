//
//  SwipeToSendView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//
import Foundation
import SwiftUI

struct SwipeToSendView: View {
    @Environment(\.colorScheme) var colorScheme

    // args
    let onConfirm: () -> Void

    // private
    @State private var offset: CGFloat = 0
    @State private var isDragging = false
    @State private var containerWidth = screenWidth

    var maxOffset: CGFloat {
        containerWidth - 70
    }

    private var height: CGFloat = 70
    private var circleSize: CGFloat {
        height
    }

    private var fontOpacity: Double {
        let percentDragged = offset / maxOffset

        if percentDragged == 0 {
            return 1
        }

        return 1.0 - percentDragged - 0.07
    }

    init(onConfirm: @escaping () -> Void) {
        self.onConfirm = onConfirm
    }

    var body: some View {
        ZStack {
            // Background capsule
            Capsule()
                .fill(Color(.systemGray5))
                .frame(height: height)

            // Blue fill that follows the drag
            GeometryReader { geometry in
                Capsule()
                    .fill(Color.midnightBtn)
                    .frame(width: offset + circleSize)
                    .frame(maxWidth: geometry.size.width, alignment: .leading)
            }
            .frame(height: height)

            // "Swipe to Send" text
            HStack {
                Spacer()
                Text("Swipe to Send")
                    .foregroundColor(colorScheme == .dark ? .white : .midnightBlue)
                    .fontWeight(.medium)
                    .opacity(fontOpacity)

                Spacer()
            }

            // Draggable button
            Circle()
                .fill(Color.midnightBtn)
                .frame(width: circleSize, height: circleSize)
                .overlay(
                    Image(systemName: "arrow.right")
                        .font(.system(size: 20))
                        .foregroundColor(.white)
                )
                .offset(x: -containerWidth / 2 + 35 + offset)
                .gesture(
                    DragGesture()
                        .onChanged { value in
                            isDragging = true
                            offset = min(maxOffset, max(0, value.translation.width))
                        }
                        .onEnded { _ in
                            isDragging = false
                            if offset > maxOffset * 0.8 {
                                // Trigger send action
                                withAnimation {
                                    offset = maxOffset
                                }

                                onConfirm()
                            } else {
                                withAnimation {
                                    offset = 0
                                }
                            }
                        }
                )
        }
        .frame(height: height)
        .onGeometryChange(for: CGRect.self) { proxy in
            proxy.frame(in: .global)
        } action: { frame in
            containerWidth = frame.width
        }
    }
}

#Preview {
    VStack {
        SwipeToSendView(onConfirm: { print("CONFIRMED") })
    }
    .padding(12)
}
