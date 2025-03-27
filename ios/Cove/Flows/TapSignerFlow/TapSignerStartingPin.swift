//
//  TapSignerStartingPin.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerStartingPin: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    var chainCode: String? = nil

    // private
    @State private var startingPin: String = ""
    @FocusState private var isFocused

    var body: some View {
        ScrollView {
            VStack(spacing: 30) {
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
                    .foregroundStyle(.white)
                    .fontWeight(.semibold)

                    Image(.tapSignerCard)
                        .offset(y: 10)
                        .clipped()
                }
                .background(Color(hex: "3A4254"))

                VStack(spacing: 20) {
                    Text("Enter Starting PIN")
                        .font(.largeTitle)
                        .fontWeight(.bold)

                    Text(
                        "The starting PIN is the 6 digit numeric PIN found of the back of your TAPSIGNER"
                    )
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal)

                HStack {
                    ForEach(0 ..< 6, id: \.self) { index in
                        Circle()
                            .stroke(.primary, lineWidth: 1.3)
                            .fill(startingPin.count <= index ? Color.clear : .primary)
                            .frame(width: 18)
                            .padding(.horizontal, 10)
                            .id(index)
                            .foregroundStyle(.primary)
                    }
                }
                .contentShape(Rectangle())
                .onTapGesture { isFocused = true }
                .fixedSize(horizontal: true, vertical: true)

                TextField("Hidden Input", text: $startingPin)
                    .opacity(0)
                    .frame(width: 0, height: 0)
                    .focused($isFocused)
                    .keyboardType(.numberPad)
            }
            .onAppear {
                startingPin = ""
                isFocused = true
            }
            .onChange(of: isFocused) { _, _ in isFocused = true }
            .onChange(of: startingPin) { old, pin in
                if pin.count == 6 {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                        manager.navigate(to:
                            .newPin(
                                tapSigner: tapSigner,
                                startingPin: pin,
                                chainCode: chainCode
                            ))
                    }
                }

                if pin.count > 6, old.count < 6 {
                    startingPin = old
                    return
                }

                if pin.count > 6 {
                    startingPin = String(startingPin.prefix(6))
                    return
                }
            }
        }
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }
}

#Preview {
    TapSignerContainer(route:
        .startingPin(
            tapSigner: tapSignerPreviewNew(preview: true),
            chainCode: nil
        ))
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
