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
    let menuItems: [MenuItem]
    let screenWidth = UIScreen.main.bounds.width

    var body: some View {
        ZStack {
            if isShowing {
                Rectangle().opacity(0.6).ignoresSafeArea().onTapGesture {
                    withAnimation {
                        isShowing = false
                    }
                }

                HStack(alignment: .top) {
                    VStack(alignment: .leading) {
                        ForEach(menuItems, id: \.destination) { item in
                            Button(action: { goTo(item) }) {
                                Label(item.title, systemImage: item.icon)
                                    .foregroundStyle(
                                        LinearGradient(colors: [Color.blue, Color.blue.opacity(0.9), Color.blue.opacity(0.8), Color.blue.opacity(0.7)],
                                                       startPoint: .topLeading, endPoint: .bottomTrailing))
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
        SidebarView(isShowing: Binding.constant(true), menuItems: [
            MenuItem(destination: RouteFactory().newWalletSelect(), title: "New Wallet", icon: "wallet.pass.fill"),
        ])
    }.background(Color.white)
}
