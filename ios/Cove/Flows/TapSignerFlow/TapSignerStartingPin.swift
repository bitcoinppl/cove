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

    let tapSigner: TapSigner
    @State var nfc: TapSignerNFC?

    @State private var startingPin = ""
    @State private var newPin = ""
    @State private var confirmPin = ""

    private var pinsMatch: Bool {
        !newPin.isEmpty && newPin == confirmPin
    }

    private func setupTapSigner() {
        if !pinsMatch { return }
        guard let nfc else { return }

        Task {
            do {
                let backup = try await nfc.setupTapSigner(startingPin, newPin)
            } catch {}
        }
    }

    var body: some View {
        VStack {
            VStack {
                Text("TapSigner Setup")
                    .font(.title)
                    .fontWeight(.bold)

                Text("Please enter your PINs")
                    .font(.body)
            }
            .padding(.horizontal, 20)

            Spacer()

            VStack(spacing: 15) {
                VStack(alignment: .leading) {
                    Text("Starting PIN")
                        .font(.subheadline)
                        .padding(.leading, 5)

                    TextField("Starting PIN", text: $startingPin)
                        .textFieldStyle(RoundedBorderTextFieldStyle())
                        .padding(.horizontal, 10)
                        .frame(height: 50)
                        .background(Color.white)
                        .cornerRadius(10)
                }

                VStack(alignment: .leading) {
                    Text("New PIN")
                        .font(.subheadline)
                        .padding(.leading, 5)

                    TextField("New PIN", text: $newPin)
                        .textFieldStyle(RoundedBorderTextFieldStyle())
                        .padding(.horizontal, 10)
                        .frame(height: 50)
                        .background(Color.white)
                        .cornerRadius(10)
                }

                VStack(alignment: .leading) {
                    Text("Confirm New PIN")
                        .font(.subheadline)
                        .padding(.leading, 5)

                    TextField("Confirm New PIN", text: $confirmPin)
                        .textFieldStyle(RoundedBorderTextFieldStyle())
                        .padding(.horizontal, 10)
                        .frame(height: 50)
                        .background(Color.white)
                        .cornerRadius(10)
                }

                if !confirmPin.isEmpty, !pinsMatch {
                    Text("PINs do not match")
                        .foregroundColor(.red)
                        .font(.caption)
                }
            }
            .padding(.bottom, 20)

            Button {
                setupTapSigner()
            } label: {
                Text("Continue")
                    .font(.title)
                    .fontWeight(.bold)
                    .foregroundColor(.white)
            }
            .padding(.horizontal, 20)
            .frame(height: 50)
            .background(pinsMatch ? Color.blue : Color.gray)
            .cornerRadius(10)
            .disabled(!pinsMatch)
        }
        .padding(.horizontal, 20)
        .padding(.top, 20)
        .navigationBarTitleDisplayMode(.inline)
        .onAppear {
            nfc = TapSignerNFC(tapcard: .tapSigner(tapSigner))
        }
    }
}

#Preview {
    TapSignerContainer(route: .startingPin(tapSignerPreviewNew(preview: true)))
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
