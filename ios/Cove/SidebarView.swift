//
//  SidebarView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SidebarView: View {
    @Environment(\.navigate) private var navigate
    @Binding var isShowing: Bool
    let currentRoute: Route

    var menuItems: [MenuItem]
    let screenWidth = UIScreen.main.bounds.width

    func setForeground(_ route: Route) -> LinearGradient {
        if RouteFactory().isSameParentRoute(route: route, routeToCheck: currentRoute) {
            return
                LinearGradient(
                    colors: [
                        Color.blue,
                        Color.blue.opacity(0.9),
                        Color.blue.opacity(0.8),
                        Color.blue.opacity(0.7)
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
        } else {
            return
                LinearGradient(
                    colors: [
                        Color.white.opacity(0.8), Color.white.opacity(0.7)
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
        }
    }

    var body: some View {
        ZStack {
            if isShowing {
                Rectangle().opacity(0.6).ignoresSafeArea().onTapGesture {
                    withAnimation {
                        isShowing = false
                    }
                }

                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 30) {
                        ForEach(menuItems, id: \.destination) { item in
                            Button(action: { goTo(item) }) {
                                Label(item.title, systemImage: item.icon)
                                    .foregroundStyle(
                                        setForeground(item.destination)
                                    )
                                    .padding(.leading, 30)
                            }
                        }
                    }
                    .frame(maxWidth: screenWidth * 0.75, maxHeight: .infinity, alignment: .leading)
                    .background(.thinMaterial)

                    Spacer()
                }
                .transition(.move(edge: .leading))
            }
        }
    }

    func goTo(_ item: MenuItem) {
        isShowing.toggle()
        navigate(item.destination)
    }
}

#Preview {
    ZStack {
        SidebarView(isShowing: Binding.constant(true), currentRoute: Route.listWallets, menuItems: MainViewModel().menuItems)
    }
    .background(Color.white)
}
