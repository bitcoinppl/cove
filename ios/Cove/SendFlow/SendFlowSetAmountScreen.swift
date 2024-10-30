//
//  SendFlowSetAmountScreen.swift
//  Cove
//
//  Created by Praveen Perera on 10/29/24.
//

import Foundation
import SwiftUI

struct SendFlowSetAmountScreen: View {
    func setToolbarAppearence() {
        let appearance = UINavigationBarAppearance()
        appearance.configureWithTransparentBackground()
        appearance.backgroundColor = UIColor.clear
        appearance.titleTextAttributes = [.foregroundColor: UIColor.white]
        appearance.largeTitleTextAttributes = [.foregroundColor: UIColor.white]

        UINavigationBar.appearance().standardAppearance = appearance
        UINavigationBar.appearance().compactAppearance = appearance
        UINavigationBar.appearance().scrollEdgeAppearance = appearance
        UINavigationBar.appearance().tintColor = .white
    }

    var body: some View {
        VStack(spacing: 0) {
            ZStack {
                VStack {
                    HStack {
                        Text("Balance")
                    }
                }
                .background(
                    Image(.headerPattern)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(width: 400, height: 300,
                               alignment: .topTrailing)
                        .clipped()
                        .ignoresSafeArea(.all)
                )
                .foregroundStyle(.white)
                .ignoresSafeArea(.all)
                .frame(width: screenWidth, height: screenHeight * 0.20)
            }
            .background(Color.midnightBlue)

            VStack {
                Text("SendFlowSetAmountScreen")
                Spacer()
            }

            Spacer()
        }
        .navigationTitle("Send")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear(perform: setToolbarAppearence)
    }
}

#Preview {
    NavigationStack {
        SendFlowSetAmountScreen()
    }
}
