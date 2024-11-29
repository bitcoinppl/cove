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

    var openPercentage: Double {
        (offset + gestureOffset) / sideBarWidth
    }

    var totalOffset: CGFloat {
        min(max(offset + gestureOffset, 0), sideBarWidth)
    }

    var body: some View {
        ZStack(alignment: .leading) {
            ZStack {
                content
                    .ignoresSafeArea(.all)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                if app.isSidebarVisible || gestureOffset > 0 || offset > 0 {
                    Rectangle()
                        .fill(Color.black)
                        .background(.black)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .opacity(openPercentage * 0.45)
                        .onTapGesture {
                            app.isSidebarVisible = false
                        }
                }
            }
            .offset(x: totalOffset)

            SidebarView(currentRoute: app.currentRoute)
                .frame(width: sideBarWidth)
                .offset(x: -sideBarWidth)
                .offset(x: totalOffset)
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
