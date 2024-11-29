//
//  SidebarContainer.swift
//  Cove
//
//  Created by Praveen Perera on 11/28/24.
//

import SwiftUI

struct SidebarContainer<Content: View>: View {
    @Environment(MainViewModel.self) private var app

    @ViewBuilder
    let content: Content

    // sidebar
    let sideBarWidth: CGFloat = 280
    @State private var offset: CGFloat = 0
    @GestureState private var gestureOffset: CGFloat = 0

    private func onDragEnded(value: DragGesture.Value) {
        let threshhold = 0.5
        let translation = value.translation.width

        if translation < 0, !app.isSidebarVisible { return }
        if translation > 0, app.isSidebarVisible { return }

        if translation > 0 {
            offset = translation
        }

        if translation < 0 {
            offset = sideBarWidth + translation
        }

        withAnimation {
            if translation > sideBarWidth * threshhold {
                offset = sideBarWidth
                app.isSidebarVisible = true
            } else {
                offset = 0
                app.isSidebarVisible = false
            }
        }
    }

    var body: some View {
        ZStack(alignment: .leading) {
            content
                .ignoresSafeArea(.all)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .offset(x: offset + gestureOffset)

            SidebarView(currentRoute: app.currentRoute)
                .frame(width: sideBarWidth)
                .offset(x: -sideBarWidth)
                .offset(x: offset + gestureOffset)
        }
        .gesture(
            DragGesture()
                .updating($gestureOffset) { value, state, _ in
                    // closed
                    if !app.isSidebarVisible,
                       value.translation.width > 0,
                       value.translation.width < sideBarWidth
                    {
                        state = value.translation.width * 0.90
                    }

                    // open
                    if app.isSidebarVisible,
                       value.translation.width < 0,
                       abs(value.translation.width) < sideBarWidth
                    {
                        state = value.translation.width * 0.90
                    }
                }
                .onEnded(onDragEnded)
        )
        .onChange(of: app.isSidebarVisible) { _, isVisible in
            withAnimation {
                offset = isVisible ? sideBarWidth : 0
            }
        }
        .ignoresSafeArea(.all)
    }
}

#Preview {
    SidebarContainer {
        VStack {}
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(LinearGradient(colors: [Color.red, Color.green], startPoint: .leading, endPoint: .trailing))
    }
    .environment(MainViewModel())
}
