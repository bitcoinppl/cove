//
//  TapSignerConfirmPin.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerConfirmPin: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let startingPin: String
    let newPin: String

    // private
    @State private var confirmPin: String = ""
    @FocusState private var isFocused

    var body: some View {
        ScrollView {
            VStack(spacing: 40) {
                VStack {
                    HStack {
                        Button(action: { manager.popRoute() }) {
                            Image(systemName: "chevron.left")
                            Text("Back")
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
                    Text("Confirm New PIN")
                        .font(.largeTitle)
                        .fontWeight(.bold)

                    Text(
                        "The PIN code is a security feature that prevents unauthorized access to your key. Please back it up and keep it safe. You'll need it for signing transactions."
                    )
                    .font(.subheadline)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal)

                HStack {
                    ForEach(0 ..< 6, id: \.self) { index in
                        Circle()
                            .stroke(.primary, lineWidth: 1.3)
                            .fill(confirmPin.count <= index ? Color.clear : .primary)
                            .frame(width: 18)
                            .padding(.horizontal, 10)
                            .id(index)
                    }
                }
                .fixedSize(horizontal: true, vertical: true)
                .contentShape(Rectangle())
                .onTapGesture { isFocused = true }

                TextField("Hidden Input", text: $confirmPin)
                    .opacity(0)
                    .frame(width: 0, height: 0)
                    .focused($isFocused)
                    .keyboardType(.numberPad)

                Spacer()
            }
            .onAppear {
                confirmPin = ""
                isFocused = true
            }
            .onChange(of: isFocused) { _, _ in isFocused = true }
            .onChange(of: confirmPin) { old, pin in
                if pin.count == 6 {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {}
                }

                if pin.count > 6, old.count < 6 {
                    confirmPin = old
                    return
                }

                if pin.count > 6 {
                    confirmPin = String(startingPin.prefix(6))
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
        route:
        .confirmPin(
            tapSigner: tapSignerPreviewNew(preview: true),
            startingPin: "123456",
            newPin: "222222"
        )
    )
    .environment(AppManager.shared)
}
