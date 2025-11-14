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

    private func onDragEnded(value: DragGesture.Value) {
        let threshold = sideBarWidth * 0.3
        let predictedEnd = value.predictedEndTranslation.width
        let currentOffset = totalOffset

        // Commit the drag position before running the snapping logic so the
        // gesture translation doesn't fight animations.
        offset = currentOffset
        dragTranslation = 0

        withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
            Log.debug(
                "onDragEnded: offset: \(offset), predictedEnd: \(predictedEnd) dragStartedWithSidebarOpen: \(dragStartedWithSidebarOpen)"
            )

            // started open
            if dragStartedWithSidebarOpen {
                // started open - closing requires dragging below 70% (196px for 280px width)
                // this means we dragged 30% towards closed
                let closeThreshold = sideBarWidth - threshold

                // predictedEnd is a translation delta, convert to absolute position
                let predictedFinalOffset = sideBarWidth + predictedEnd

                // require BOTH current offset AND predicted end to be below threshold
                // this prevents accidental closes from small drags with high predicted velocity
                if offset < closeThreshold, predictedFinalOffset < closeThreshold {
                    // snap to closed
                    offset = 0
                    app.isSidebarVisible = false
                    dragStartedWithSidebarOpen = false
                } else {
                    // snap back to open
                    offset = sideBarWidth
                    app.isSidebarVisible = true
                }
            }

            // started closed
            if !dragStartedWithSidebarOpen {
                // started closed - opening requires dragging past 30% (84px for 280px width)
                if offset > threshold || predictedEnd > threshold {
                    // snap to open
                    offset = sideBarWidth
                    app.isSidebarVisible = true
                    dragStartedWithSidebarOpen = true
                } else {
                    // snap back to closed
                    offset = 0
                    app.isSidebarVisible = false
                }
            }

            isDragging = false
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

            if app.isSidebarVisible {
                Rectangle()
                    .fill(Color.black)
                    .background(.black)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .opacity(openPercentage * 0.45)
                    .onTapGesture {
                        withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
                            offset = 0
                            dragTranslation = 0
                            app.isSidebarVisible = false
                            dragStartedWithSidebarOpen = false
                        }
                    }
                    .gesture(
                        DragGesture(minimumDistance: 5)
                            .onChanged { value in
                                isDragging = true
                                dragStartedWithSidebarOpen = true

                                let translation = closingTranslation(from: value)
                                dragTranslation = translation

                                Log.debug(
                                    "OVERLAY: translation=\(translation), totalOffset=\(totalOffset), baseOffset=\(offset), isSidebarVisible=\(app.isSidebarVisible)"
                                )
                            }
                            .onEnded(onDragEnded)
                    )
                    .ignoresSafeArea(.all)
                    .zIndex(1)
            }

            SidebarView(currentRoute: app.currentRoute)
                .frame(width: sideBarWidth)
                .offset(x: -sideBarWidth)
                .offset(x: totalOffset)
                .zIndex(2)
                .simultaneousGesture(
                    DragGesture(minimumDistance: 5)
                        .onChanged { value in
                            guard app.isSidebarVisible else { return }

                            isDragging = true
                            dragStartedWithSidebarOpen = true

                            let translation = closingTranslation(from: value)
                            dragTranslation = translation

                            Log.debug(
                                "SIDEBAR: translation=\(translation), totalOffset=\(totalOffset), baseOffset=\(offset), isSidebarVisible=\(app.isSidebarVisible)"
                            )
                        }
                        .onEnded(onDragEnded)
                )

            // edge handle for opening sidebar when closed
            if !app.isSidebarVisible, app.router.routes.isEmpty {
                Color.clear
                    .frame(width: 24)
                    .frame(maxHeight: .infinity)
                    .contentShape(Rectangle())
                    .gesture(
                        DragGesture(minimumDistance: 5)
                            .onChanged { value in
                                isDragging = true
                                dragStartedWithSidebarOpen = false

                                let translation = value.translation.width
                                let translationHeight = value.translation.height

                                // only handle horizontal gestures
                                if abs(translationHeight) > abs(translation) {
                                    dragTranslation = 0
                                    return
                                }

                                // only allow opening (positive translation)
                                if translation < 0 {
                                    dragTranslation = 0
                                    return
                                }

                                let adjustedTranslation = min(max(translation * 0.95, 0), sideBarWidth)
                                dragTranslation = adjustedTranslation

                                Log.debug(
                                    "EDGE HANDLE: translation=\(translation), totalOffset=\(totalOffset), baseOffset=\(offset), isSidebarVisible=\(app.isSidebarVisible)"
                                )
                            }
                            .onEnded(onDragEnded)
                    )
            }
        }
        .onAppear {
            offset = app.isSidebarVisible ? sideBarWidth : 0
            dragStartedWithSidebarOpen = app.isSidebarVisible
        }
        .onChange(of: app.isSidebarVisible) { _, isVisible in
            guard !isDragging else { return }

            withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
                offset = isVisible ? sideBarWidth : 0
                dragTranslation = 0
                dragStartedWithSidebarOpen = isVisible
            }
        }
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
