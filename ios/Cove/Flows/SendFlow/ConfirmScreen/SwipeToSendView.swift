//
//  SwipeToSendView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//
import Foundation
import SwiftUI

private let swipeToSendVerticalTextPadding: CGFloat = 48
private let swipeToSendMinimumTrailingTextPadding: CGFloat = 16

enum SendState: Hashable, Equatable {
    case idle
    case sending
    case payjoinWaiting(deadlineSecs: UInt64)
    case sent
    case error(String)
}

struct SwipeToSendView: View {
    static let minimumControlHeight: CGFloat = 70

    @Environment(\.colorScheme) var colorScheme

    // args
    @Binding var sendState: SendState
    let onConfirm: () -> Void

    // private
    @State var offset: CGFloat = 0
    @State var isDragging = false
    @State var containerWidth = screenWidth
    @State private var measuredTextHeight: CGFloat = 0

    var maxOffset: CGFloat {
        max(0, containerWidth - circleSize)
    }

    var height: CGFloat {
        max(Self.minimumControlHeight, measuredTextHeight + swipeToSendVerticalTextPadding)
    }

    var circleSize: CGFloat {
        height
    }

    var fontOpacity: Double {
        guard maxOffset > 0 else { return 1 }

        let percentDragged = offset / maxOffset

        if percentDragged == 0 {
            return 1
        }

        return 1.0 - percentDragged - 0.07
    }

    func onChangeSendState(_: SendState, _ state: SendState) {
        // set to full
        if state != .idle { offset = maxOffset }

        // reset
        if state == .idle { offset = 0 }
    }

    var body: some View {
        ZStack {
            SwipeToSendBackground(
                fillWidth: offset + circleSize,
                height: height
            )

            SwipeToSendPrompt(
                circleSize: circleSize,
                fontColor: colorScheme == .dark ? .white : .midnightBtn,
                fontOpacity: fontOpacity
            )

            SwipeToSendHandle(
                sendState: sendState,
                offset: $offset,
                isDragging: $isDragging,
                circleSize: circleSize,
                containerWidth: containerWidth,
                maxOffset: maxOffset,
                onConfirm: onConfirm
            )

            SwipeToSendStatus(sendState: $sendState)
        }
        .frame(height: height)
        .onChange(of: sendState, initial: true, onChangeSendState)
        .onPreferenceChange(SwipeToSendTextHeightPreferenceKey.self) { height in
            measuredTextHeight = height
        }
        .onGeometryChange(for: CGRect.self) { proxy in
            proxy.frame(in: .global)
        } action: { frame in
            containerWidth = frame.width
        }
    }
}

private struct SwipeToSendBackground: View {
    let fillWidth: CGFloat
    let height: CGFloat

    var body: some View {
        ZStack {
            Capsule()
                .fill(Color(.systemGray5))
                .frame(height: height)

            GeometryReader { geometry in
                Capsule()
                    .fill(Color.midnightBtn)
                    .frame(width: fillWidth)
                    .frame(maxWidth: geometry.size.width, alignment: .leading)
            }
            .frame(height: height)
        }
    }
}

private struct SwipeToSendPrompt: View {
    let circleSize: CGFloat
    let fontColor: Color
    let fontOpacity: Double

    var body: some View {
        HStack(spacing: 0) {
            Color.clear
                .frame(width: circleSize + swipeToSendMinimumTrailingTextPadding)

            Text("Swipe to Send")
                .foregroundColor(fontColor)
                .fontWeight(.medium)
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity)
                .opacity(fontOpacity)
                .recordSwipeToSendTextHeight()

            Color.clear
                .frame(width: swipeToSendMinimumTrailingTextPadding)
        }
    }
}

private struct SwipeToSendHandle: View {
    let sendState: SendState
    @Binding var offset: CGFloat
    @Binding var isDragging: Bool

    let circleSize: CGFloat
    let containerWidth: CGFloat
    let maxOffset: CGFloat
    let onConfirm: () -> Void

    var body: some View {
        Circle()
            .fill(Color.midnightBtn)
            .frame(width: circleSize, height: circleSize)
            .overlay {
                if sendState == .idle {
                    PulsingSendArrow(isPulsing: !isDragging && offset == 0)
                }
            }
            .offset(x: -containerWidth / 2 + circleSize / 2 + offset)
            .gesture(
                DragGesture()
                    .onChanged(dragChanged)
                    .onEnded(dragEnded)
            )
    }

    private func dragChanged(_ value: DragGesture.Value) {
        isDragging = true
        offset = min(maxOffset, max(0, value.translation.width))
    }

    private func dragEnded(_: DragGesture.Value) {
        isDragging = false

        guard offset > maxOffset * 0.8 else {
            withAnimation {
                offset = 0
            }
            return
        }

        withAnimation {
            offset = maxOffset
        }

        onConfirm()
    }
}

private struct SwipeToSendStatus: View {
    @Binding var sendState: SendState

    var body: some View {
        Group {
            switch sendState {
            case .idle:
                EmptyView()
            case .sending, .payjoinWaiting:
                SwipeToSendSendingStatus()
            case .sent:
                SwipeToSendSentStatus()
            case .error:
                SwipeToSendErrorStatus(sendState: $sendState)
            }
        }
        .foregroundStyle(.white)
    }
}

private struct SwipeToSendSendingStatus: View {
    var body: some View {
        HStack(spacing: 16) {
            Text("sending")
                .recordSwipeToSendTextHeight()
            ThreeDotsAnimation()
        }
    }
}

private struct SwipeToSendSentStatus: View {
    var body: some View {
        HStack(spacing: 12) {
            Text("sent")
                .recordSwipeToSendTextHeight()
            Image(systemName: "checkmark")
                .foregroundColor(.green)
        }
    }
}

private struct SwipeToSendErrorStatus: View {
    @Binding var sendState: SendState

    var body: some View {
        HStack(spacing: 10) {
            Text("error")
                .recordSwipeToSendTextHeight()
            Image(systemName: "xmark")
                .foregroundColor(.red)
                .onAppear(perform: resetAfterDelay)
        }
    }

    private func resetAfterDelay() {
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
            sendState = .idle
        }
    }
}

private struct SwipeToSendTextHeightPreferenceKey: PreferenceKey {
    static let defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

private extension View {
    func recordSwipeToSendTextHeight() -> some View {
        background {
            GeometryReader { proxy in
                Color.clear.preference(
                    key: SwipeToSendTextHeightPreferenceKey.self,
                    value: proxy.size.height
                )
            }
        }
    }
}

private struct PulsingSendArrow: View {
    let isPulsing: Bool

    @State private var pulseStartedAt: Date?

    var body: some View {
        TimelineView(.animation(paused: !isPulsing)) { context in
            Image(systemName: "arrow.right")
                .font(.system(size: 20))
                .foregroundColor(.white)
                .opacity(opacity(at: context.date))
        }
        .onChange(of: isPulsing, initial: true) { _, pulsing in
            pulseStartedAt = pulsing ? Date() : nil
        }
    }

    private func opacity(at date: Date) -> Double {
        guard isPulsing, let pulseStartedAt else { return 1 }

        let elapsed = max(0, date.timeIntervalSince(pulseStartedAt))
        let progress = elapsed
            .truncatingRemainder(dividingBy: 1.8) / 0.9
        let mirroredProgress = progress <= 1 ? progress : 2 - progress
        let easedProgress = 0.5 - (0.5 * cos(mirroredProgress * .pi))

        return 1 - (0.4 * easedProgress)
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
        @State var sendState: SendState = .error("no work")

        var body: some View {
            SwipeToSendView(
                sendState: $sendState,
                onConfirm: { print("CONFIRMED") }
            )
        }
    }

    return Container()
}
