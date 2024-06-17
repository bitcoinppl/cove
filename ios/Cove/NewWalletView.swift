//
//  NewWalletView.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

struct NewWalletView: View {
    var body: some View {
        VStack {
            HStack {
                Text("How do you want to secure your Bitcoin?")
                    .font(.largeTitle)
                    .multilineTextAlignment(.center)
                    .foregroundColor(.white)
            }.padding(.top, 30)
                .padding(.bottom, 20)
                .padding(.horizontal, 30)
            Spacer()
            HStack {
                Spacer()
                Text("On This Device").font(.title)
                Spacer()
            }
            .cornerRadius(2.0)
            .frame(maxHeight: .infinity)
            .background(
                RoundedRectangle(cornerRadius: 15)
                    .fill(Color.blue)
                    .brightness(-0.1)
            )
            .padding(.vertical, 30)
            .padding(.horizontal, 40)
            .foregroundColor(.white)
            Spacer()
            HStack {
                Spacer()
                Text("On Hardware Wallet").font(.title)
                Spacer()
            }
            .frame(maxHeight: .infinity)
            .background(
                RoundedRectangle(cornerRadius: 15)
                    .fill(Color.green)
                    .brightness(-0.15)
            )
            .foregroundColor(.white)
            .padding(.vertical, 30)
            .padding(.horizontal, 40)
            Spacer()
        }.background(.black)
    }
}

#Preview {
    NewWalletView()
}
