//
//  SwipeToSendView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//
import Foundation
import SwiftUI

enum SendState: Hashable, Equatable {
    case idle
    case sending
    case sent
    case error
}

struct SwipeToSendView: View {
    @Environment(\.colorScheme) var colorScheme

    // args
    @Binding var sendState: SendState
    let onConfirm: () -> Void

    // private
    @State var offset: CGFloat = 0
    @State var isDragging = false
    @State var containerWidth = screenWidth

    var maxOffset: CGFloat {
        containerWidth - 70
    }

    var height: CGFloat = 70
    var circleSize: CGFloat {
        height
    }

    var fontOpacity: Double {
        let percentDragged = offset / maxOffset

        if percentDragged == 0 {
            return 1
        }

        return 1.0 - percentDragged - 0.07
    }

    func onChangeSendState(_: SendState, _ state: SendState) {
        // set to full
        if state != .idle { offset = maxOffset }
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
                    .foregroundColor(colorScheme == .dark ? .white : .midnightBtn)
                    .fontWeight(.medium)
                    .opacity(fontOpacity)

                Spacer()
            }

            // Draggable button
            Circle()
                .fill(Color.midnightBtn)
                .frame(width: circleSize, height: circleSize)
                .overlay(
                    Group {
                        if sendState == .idle {
                            Image(systemName: "arrow.right")
                                .font(.system(size: 20))
                                .foregroundColor(.white)
                        }
                    }
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

            // Text overlay when sending/sent/error
            Group {
                switch sendState {
                case .idle: EmptyView()
                case .sending: HStack(spacing: 16) {
                        Text("sending")
                        ThreeDotsAnimation()
                    }
                case .sent: HStack(spacing: 12) {
                        Text("sent")
                        Image(systemName: "checkmark")
                            .foregroundColor(.green)
                    }
                case .error: HStack(spacing: 10) {
                        Text("error")
                        Image(systemName: "xmark")
                            .foregroundColor(.red)
                            .onAppear {
                                DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                                    sendState = .idle
                                }
                            }
                    }
                }
            }.foregroundStyle(.white)
        }
        .frame(height: height)
        .onChange(of: sendState, initial: true, onChangeSendState)
        .onGeometryChange(for: CGRect.self) { proxy in
            proxy.frame(in: .global)
        } action: { frame in
            containerWidth = frame.width
        }
    }
}

#Preview("idle") {
    SwipeToSendView(
        sendState: Binding.constant(.idle),
        onConfirm: { print("CONFIRMED") }
    )
}

#Preview("sending") {
    SwipeToSendView(
        sendState: Binding.constant(.sending),
        onConfirm: { print("CONFIRMED") }
    )
}

#Preview("sent") {
    SwipeToSendView(
        sendState: Binding.constant(.sent),
        onConfirm: { print("CONFIRMED") }
    )
}

#Preview("error") {
    struct Container: View {
        @State var sendState: SendState = .error

        var body: some View {
            SwipeToSendView(
                sendState: $sendState,
                onConfirm: { print("CONFIRMED") }
            )
        }
    }

    return Container()
}
