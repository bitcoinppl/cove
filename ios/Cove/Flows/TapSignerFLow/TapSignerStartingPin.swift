//
//  TapSignerStartingPin.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerStartingPin: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth

    @State private var pin = ""
    @State private var pinConfirm = ""

    var body: some View {
        VStack {
            Spacer()

            Text("TapSigner")
                .font(.largeTitle)
                .fontWeight(.bold)

            Spacer()

            VStack {
                Text("Starting PIN")
                    .font(.title)
                    .fontWeight(.bold)

                Text("Please enter your PIN")
                    .font(.body)
            }
            .padding(.horizontal, 20)

            HStack {
                TextField("PIN", text: $pin)
                    .textFieldStyle(RoundedBorderTextFieldStyle())
                    .padding(.horizontal, 10)
                    .frame(height: 50)
                    .background(Color.white)
                    .cornerRadius(10)

                Spacer()
            }

            Button {
//                app.dispatch(action: .tapSignerStartingPin(pin: pin))
            } label: {
                Text("Continue")
                    .font(.title)
                    .fontWeight(.bold)
                    .foregroundColor(.white)
            }
            .padding(.horizontal, 20)
            .frame(height: 50)
            .background(Color.blue)
            .cornerRadius(10)
        }
        .padding(.horizontal, 20)
        .padding(.top, 20)
        .navigationBarTitleDisplayMode(.inline)
    }
}

#Preview {
    TapSignerContainer(route: .startingPin)
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
