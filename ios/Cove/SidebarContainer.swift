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
    @State private var dragStartedWithSidebarOpen = false

    private func onDragEnded(value: DragGesture.Value) {
        let threshold = sideBarWidth * 0.3
        let predictedEnd = value.predictedEndTranslation.width

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
                if offset < closeThreshold && predictedFinalOffset < closeThreshold {
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
        }
    }

    var openPercentage: Double {
        offset / sideBarWidth
    }

    var totalOffset: CGFloat {
        min(max(offset, 0), sideBarWidth)
    }

    var body: some View {
        ZStack(alignment: .leading) {
            ZStack {
                content
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                if app.isSidebarVisible || offset > 0 {
                    Rectangle()
                        .fill(Color.black)
                        .background(.black)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .opacity(openPercentage * 0.45)
                        .onTapGesture {
                            app.isSidebarVisible = false
                        }
                        .ignoresSafeArea(.all)
                }
            }
            .offset(x: totalOffset)

            SidebarView(currentRoute: app.currentRoute)
                .frame(width: sideBarWidth)
                .offset(x: -sideBarWidth)
                .offset(x: totalOffset)
        }
        .highPriorityGesture(
            !app.router.routes.isEmpty
                ? nil
                : DragGesture(minimumDistance: 5)
                    .onChanged { value in
                        // capture initial state on first update (when drag just started)
                        if abs(value.translation.width) < 5 && abs(value.translation.height) < 5 {
                            dragStartedWithSidebarOpen = app.isSidebarVisible
                        }

                        // only activate from leading edge when sidebar was closed at start
                        if value.startLocation.x > 25, !dragStartedWithSidebarOpen {
                            return
                        }

                        let translation = value.translation.width
                        let translationHeight = value.translation.height

                        // only handle horizontal gestures, let vertical scrolls pass through
                        if abs(translationHeight) > abs(translation) {
                            return
                        }

                        // calculate offset based on state at drag START (not current state)
                        let newOffset: CGFloat
                        if dragStartedWithSidebarOpen {
                            // started with sidebar open - calculate from open position
                            newOffset = min(
                                max(sideBarWidth + (translation * 0.95), 0), sideBarWidth)
                        } else {
                            // started with sidebar closed - calculate from edge
                            newOffset = min(max(translation * 0.95, 0), sideBarWidth)
                        }

                        offset = newOffset
                    }
                    .onEnded(onDragEnded)
        )
        .onChange(of: app.isSidebarVisible) { _, isVisible in
            withAnimation {
                offset = isVisible ? sideBarWidth : 0
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
