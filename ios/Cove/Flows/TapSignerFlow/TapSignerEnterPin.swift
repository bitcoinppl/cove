//
//  TapSignerEnterPin.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerEnterPin: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let message: String
    let cmd: TapSignerCmd

    // private
    @State private var pin: String = ""
    @FocusState private var isFocused

    var body: some View {
        ScrollView {
            VStack(spacing: 40) {
                VStack {
                    HStack {
                        Button(action: { app.sheetState = .none }) {
                            Text("Cancel")
                        }

                        Spacer()
                    }
                    .padding(.top, 20)
                    .padding(.horizontal, 10)
                    .foregroundStyle(.primary)
                    .fontWeight(.semibold)

                    Image(systemName: "lock")
                        .font(.system(size: 100))
                        .foregroundColor(.blue)
                        .padding(.top, 22)
                }

                VStack(spacing: 20) {
                    Text("Enter PIN")
                        .font(.largeTitle)
                        .fontWeight(.bold)

                    Text(message)
                        .font(.subheadline)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal)

                HStack {
                    ForEach(0..<6, id: \.self) { index in
                        Circle()
                            .stroke(.primary, lineWidth: 1.3)
                            .fill(pin.count <= index ? Color.clear : .primary)
                            .frame(width: 18)
                            .padding(.horizontal, 10)
                            .id(index)
                    }
                }
                .fixedSize(horizontal: true, vertical: true)
                .contentShape(Rectangle())
                .onTapGesture { isFocused = true }

                TextField("Hidden Input", text: $pin)
                    .opacity(0)
                    .frame(width: 0, height: 0)
                    .focused($isFocused)
                    .keyboardType(.numberPad)

                Spacer()
            }
            .onAppear {
                pin = ""
                isFocused = true
            }
            .onChange(of: isFocused) { _, _ in isFocused = true }
            .onChange(of: pin) { old, newPin in
                if newPin.count == 6 {
                    manager.enteredPin = newPin
                    let nfc = TapSignerNFC(tapSigner)
                    manager.nfc = nfc

                    Task {
                        switch await nfc.derive(pin: newPin) {
                        case let .success(deriveInfo):
                            manager.resetRoute(to: .importSuccess(tapSigner, deriveInfo))
                        case let .failure(error):
                            if error.isAuthError {
                                app.alertState = .init(.tapSignerInvalidAuth)
                            } else {
                                app.alertState = .init(.tapSignerDeriveFailed(error.describe))
                            }
                        }
                    }

                    pin = ""
                }

                if newPin.count > 6, old.count < 6 {
                    pin = old
                    return
                }

                if newPin.count > 6 {
                    pin = String(pin.prefix(6))
                    return
                }
            }
        }
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }
}

#Preview {
    TapSignerContainer(
        route: .enterPin(
            tapSigner: tapSignerPreviewNew(preview: true),
            userMessage:
                "For security purposes, you need to enter your TAPSIGNER PIN before you can import your wallet",
            cmd: .derive(pin: "012345")
        )
    )
    .environment(AppManager.shared)
    .environment(AuthManager.shared)
}
