//
//  SendFlowHeaderView.swift
//  Cove
//
//  Created by Praveen Perera on 10/30/24.
//

import Foundation
import SwiftUI

struct SendFlowHeaderView: View {
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
        VStack {
            HStack {
                Text("Balance")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Spacer()
            }

            HStack {
                Text("5,215,310")
                    .font(.title2)
                    .fontWeight(.bold)
                Text("sats")
                    .font(.subheadline)
                Spacer()

                Image(systemName: "eye.slash")
            }
            .padding(.top, 2)
        }
        .padding()
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
        .background(Color.midnightBlue)
        .onAppear(perform: setToolbarAppearence)
    }
}

#Preview {
    SendFlowHeaderView()
}
