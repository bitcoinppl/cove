//
//  SidebarContainer.swift
//  Cove
//
//  Created by Praveen Perera on 11/28/24.
//

import SwiftUI

struct SidebarContainer<Content: View>: View {
    @Environment(AppManager.self) private var app

    @ViewBuilder
    let content: Content

    // sidebar
    let sideBarWidth: CGFloat = 280
    @State private var offset: CGFloat = 0
    @State private var dragTranslation: CGFloat = 0
    @State private var dragStartedWithSidebarOpen = false
    @State private var isDragging = false

    private func closingTranslation(from value: DragGesture.Value) -> CGFloat {
        let translation = value.translation.width * 0.95
        return max(min(translation, 0), -sideBarWidth)
    }

    private func isVerticalDominantDrag(_ value: DragGesture.Value) -> Bool {
        abs(value.translation.height) > abs(value.translation.width)
    }

    private func syncSidebarState(isVisible: Bool) {
        offset = isVisible ? sideBarWidth : 0
        dragTranslation = 0
        dragStartedWithSidebarOpen = isVisible
        isDragging = false
    }

    private func updateSidebarState(isVisible: Bool, animated: Bool = true) {
        let update = {
            syncSidebarState(isVisible: isVisible)
        }

        if animated {
            withAnimation(.spring(response: 0.3, dampingFraction: 0.8), update)
        } else {
            update()
        }

        app.isSidebarVisible = isVisible
    }

    private func onDragEnded(value: DragGesture.Value) {
        guard isDragging else { return }

        // if sidebar was closed programmatically during this gesture
        // (onChange already reset offset and dragTranslation to 0),
        // don't let stale gesture data reopen it
        if !app.isSidebarVisible, offset == 0, dragTranslation == 0 {
            isDragging = false
            return
        }

        let threshold = sideBarWidth * 0.3
        let predictedEnd = value.predictedEndTranslation.width
        let currentOffset = totalOffset
        let startedOpen = dragStartedWithSidebarOpen

        // Commit the drag position before running the snapping logic so the
        // gesture translation doesn't fight animations.
        offset = currentOffset
        dragTranslation = 0

        withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
            if startedOpen {
                // started open - closing requires dragging below 70% (196px for 280px width)
                // this means we dragged 30% towards closed
                let closeThreshold = sideBarWidth - threshold

                // predictedEnd is a translation delta, convert to absolute position
                let predictedFinalOffset = sideBarWidth + predictedEnd

                // require BOTH current offset AND predicted end to be below threshold
                // this prevents accidental closes from small drags with high predicted velocity
                if offset < closeThreshold, predictedFinalOffset < closeThreshold {
                    // snap to closed
                    updateSidebarState(isVisible: false, animated: false)
                } else {
                    // snap back to open
                    updateSidebarState(isVisible: true, animated: false)
                }
            } else {
                // started closed - opening requires dragging past 30% (84px for 280px width)
                if offset > threshold || predictedEnd > threshold {
                    // snap to open
                    updateSidebarState(isVisible: true, animated: false)
                } else {
                    // snap back to closed
                    updateSidebarState(isVisible: false, animated: false)
                }
            }
        }
    }

    var openPercentage: Double {
        totalOffset / sideBarWidth
    }

    var totalOffset: CGFloat {
        min(max(offset + dragTranslation, 0), sideBarWidth)
    }

    var body: some View {
        ZStack(alignment: .leading) {
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .offset(x: totalOffset)

            SidebarBackdrop(
                isVisible: app.isSidebarVisible,
                openPercentage: openPercentage,
                dragTranslation: $dragTranslation,
                dragStartedWithSidebarOpen: $dragStartedWithSidebarOpen,
                isDragging: $isDragging,
                close: { updateSidebarState(isVisible: false) },
                isVerticalDominantDrag: isVerticalDominantDrag,
                closingTranslation: closingTranslation,
                onDragEnded: onDragEnded
            )

            SidebarPanel(
                currentRoute: app.currentRoute,
                width: sideBarWidth,
                offset: totalOffset,
                isVisible: app.isSidebarVisible,
                dragTranslation: $dragTranslation,
                dragStartedWithSidebarOpen: $dragStartedWithSidebarOpen,
                isDragging: $isDragging,
                isVerticalDominantDrag: isVerticalDominantDrag,
                closingTranslation: closingTranslation,
                onDragEnded: onDragEnded
            )

            SidebarOpeningHandle(
                isVisible: !app.isSidebarVisible && app.router.routes.isEmpty,
                sideBarWidth: sideBarWidth,
                dragTranslation: $dragTranslation,
                dragStartedWithSidebarOpen: $dragStartedWithSidebarOpen,
                isDragging: $isDragging,
                onDragEnded: onDragEnded
            )
        }
        .onAppear {
            syncSidebarState(isVisible: app.isSidebarVisible)
        }
        .onChange(of: app.isSidebarVisible) { _, isVisible in
            if isVisible { app.loadWallets() }

            // when closing programmatically (e.g. button tap in sidebar),
            // a simultaneousGesture may have set isDragging from a slight
            // finger movement — reset it so the close animation runs
            if !isVisible { isDragging = false }

            if isVisible, isDragging { return }

            updateSidebarState(isVisible: isVisible)
        }
    }
}

private struct SidebarBackdrop: View {
    let isVisible: Bool
    let openPercentage: Double
    @Binding var dragTranslation: CGFloat
    @Binding var dragStartedWithSidebarOpen: Bool
    @Binding var isDragging: Bool
    let close: () -> Void
    let isVerticalDominantDrag: (DragGesture.Value) -> Bool
    let closingTranslation: (DragGesture.Value) -> CGFloat
    let onDragEnded: (DragGesture.Value) -> Void

    var body: some View {
        if isVisible {
            Rectangle()
                .fill(Color.black)
                .background(.black)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .opacity(openPercentage * 0.45)
                .onTapGesture(perform: close)
                .gesture(
                    DragGesture(minimumDistance: 5)
                        .onChanged { value in
                            guard !isVerticalDominantDrag(value) else {
                                dragTranslation = 0
                                return
                            }

                            isDragging = true
                            dragStartedWithSidebarOpen = true
                            dragTranslation = closingTranslation(value)
                        }
                        .onEnded(onDragEnded)
                )
                .ignoresSafeArea(.all)
                .zIndex(1)
        }
    }
}

private struct SidebarPanel: View {
    let currentRoute: Route
    let width: CGFloat
    let offset: CGFloat
    let isVisible: Bool
    @Binding var dragTranslation: CGFloat
    @Binding var dragStartedWithSidebarOpen: Bool
    @Binding var isDragging: Bool
    let isVerticalDominantDrag: (DragGesture.Value) -> Bool
    let closingTranslation: (DragGesture.Value) -> CGFloat
    let onDragEnded: (DragGesture.Value) -> Void

    var body: some View {
        SidebarView(currentRoute: currentRoute)
            .frame(width: width)
            .offset(x: -width)
            .offset(x: offset)
            .zIndex(2)
            .simultaneousGesture(
                DragGesture(minimumDistance: 5)
                    .onChanged { value in
                        guard isVisible else { return }
                        guard !isVerticalDominantDrag(value) else {
                            dragTranslation = 0
                            return
                        }

                        isDragging = true
                        dragStartedWithSidebarOpen = true
                        dragTranslation = closingTranslation(value)
                    }
                    .onEnded(onDragEnded)
            )
    }
}

private struct SidebarOpeningHandle: View {
    let isVisible: Bool
    let sideBarWidth: CGFloat
    @Binding var dragTranslation: CGFloat
    @Binding var dragStartedWithSidebarOpen: Bool
    @Binding var isDragging: Bool
    let onDragEnded: (DragGesture.Value) -> Void

    var body: some View {
        if isVisible {
            Color.clear
                .frame(width: 24)
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 5)
                        .onChanged(updateDrag)
                        .onEnded(onDragEnded)
                )
        }
    }

    private func updateDrag(_ value: DragGesture.Value) {
        let translation = value.translation.width
        let translationHeight = value.translation.height

        guard abs(translationHeight) <= abs(translation), translation >= 0 else {
            dragTranslation = 0
            return
        }

        isDragging = true
        dragStartedWithSidebarOpen = false
        dragTranslation = min(max(translation * 0.95, 0), sideBarWidth)
    }
}

#Preview {
    SidebarContainer {
        VStack {}
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(
                LinearGradient(
                    colors: [Color.red, Color.yellow],
                    startPoint: .leading,
                    endPoint: .trailing
                )
            )
    }
    .environment(AppManager.shared)
}
