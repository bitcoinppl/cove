//
//  SidebarView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SidebarView: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    @Binding var isShowing: Bool

    let currentRoute: Route

    @GestureState private var dragState = CGSize.zero
    @State private var sidebarOffset = -1 * UIScreen.main.bounds.width
    private let screenWidth = UIScreen.main.bounds.width

    func setForeground(_ route: Route) -> LinearGradient {
        if RouteFactory().isSameParentRoute(route: route, routeToCheck: currentRoute) {
            LinearGradient(
                colors: [
                    Color.blue,
                    Color.blue.opacity(0.9),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        } else {
            LinearGradient(
                colors: [
                    Color.primary.opacity(0.8), Color.primary.opacity(0.7),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        }
    }

    var body: some View {
        ZStack {
            if sidebarOffset == 0 {
                Rectangle()
                    .ignoresSafeArea()
                    .foregroundColor(.black)
                    .opacity(0.95)
                    .onTapGesture {
                        withAnimation {
                            isShowing = false
                        }
                    }
            }

            HStack(alignment: .top) {
                VStack(spacing: 40) {
                    Spacer()

                    Button(action: { goTo(RouteFactory().newWalletSelect()) }) {
                        Label("Add Wallet", systemImage: "wallet.pass.fill")
                            .foregroundStyle(.white)
                            .font(.headline)
                            .frame(minWidth: screenWidth * 0.55, minHeight: 45)
                            .background(Color.blue)
                            .cornerRadius(10)
                    }

                    if app.numberOfWallets > 1 {
                        Button(action: { goTo(Route.listWallets) }) {
                            Label("Change Wallet", systemImage: "arrow.uturn.right.square.fill")
                                .foregroundStyle(.white)
                                .font(.headline)
                                .frame(minWidth: screenWidth * 0.55, minHeight: 45)
                                .background(Color.blue)
                                .cornerRadius(10)
                        }
                    }

                    Spacer()
                    HStack(alignment: .center) {
                        Button(
                            action: { goTo(.settings) },
                            label: {
                                HStack {
                                    Image(systemName: "gear")
                                        .foregroundStyle(Color.white.gradient.opacity(0.5))

                                    Text("Settings")
                                        .foregroundStyle(Color.white.gradient)
                                }
                            }
                        )
                        .frame(maxWidth: screenWidth * 0.75)
                    }
                }
                .frame(maxWidth: screenWidth * 0.75, maxHeight: .infinity, alignment: .leading)
                .background(
                    LinearGradient(
                        gradient:
                        Gradient(colors: [Color.blue.opacity(1), Color.blue.opacity(0.75)]),
                        startPoint: .bottomTrailing, endPoint: .topLeading
                    )
                )
                Spacer()
            }
            .transition(.move(edge: .leading))
        }
        .gesture(
            DragGesture()
                .updating($dragState) { value, state, _ in
                    state = CGSize(width: value.translation.width, height: 0)
                }
                .onEnded { gesture in
                    let dragThreshold: CGFloat = 100
                    let draggedRatio = -gesture.translation.width / screenWidth

                    withAnimation(.spring()) {
                        if draggedRatio > 0.5
                            || gesture.predictedEndTranslation.width < -dragThreshold
                        {
                            sidebarOffset = -screenWidth
                            isShowing = false
                        } else {
                            sidebarOffset = 0
                            isShowing = true
                        }
                    }
                }
        )
        .onChange(of: isShowing) { _, newValue in
            withAnimation {
                sidebarOffset = newValue ? 0 : -1 * screenWidth
            }
        }
        .onAppear {
            if isShowing {
                sidebarOffset = 0
            }
        }
        .offset(x: sidebarOffset)
    }

    func goTo(_ route: Route) {
        isShowing = false

        if !app.hasWallets, route == Route.newWallet(.select) {
            return app.resetRoute(to: RouteFactory().newWalletSelect())
        } else {
            navigate(route)
        }
    }
}

#Preview {
    ZStack {
        SidebarView(isShowing: Binding.constant(true), currentRoute: Route.listWallets)
    }
    .environment(MainViewModel())
    .background(Color.white)
}
