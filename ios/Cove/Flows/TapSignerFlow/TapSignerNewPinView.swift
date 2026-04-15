//
//  TapSignerNewPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerNewPinView: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let args: TapSignerNewPinArgs

    // private
    @State private var newPin: String = ""
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
                    Text("Create New PIN")
                        .font(.largeTitle)
                        .fontWeight(.bold)

                    Text(
                        "The PIN code (6-32 characters) is a security feature that prevents unauthorized access to your key. Please back it up and keep it safe. You'll need it for signing transactions."
                    )
                    .font(.subheadline)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal)

                let columns = Array(repeating: GridItem(.flexible(), spacing: 12, alignment: .center), count: 6)
                LazyVGrid(columns: columns, alignment: .center, spacing: 12) {
                    ForEach(0 ..< min(max(newPin.count, 6), 32), id: \.self) { index in
                        Circle()
                            .stroke(.primary, lineWidth: 1.3)
                            .background(
                                Circle()
                                    .fill(newPin.count <= index ? Color.clear : .primary)
                            )
                            .frame(width: 18, height: 18)
                            .id(index)
                    }
                }
                .padding(.horizontal, 36)
                .frame(maxWidth: .infinity, alignment: .center)
                .contentShape(Rectangle())
                .onTapGesture { isFocused = true }

                Text("\(newPin.count)/32 characters")
                    .font(.caption)
                    .foregroundStyle(.gray)
                    .frame(maxWidth: .infinity, alignment: .center)

                TextField("Hidden Input", text: $newPin)
                    .opacity(0)
                    .frame(width: 0, height: 0)
                    .focused($isFocused)
                    .keyboardType(.numberPad)

                Button(action: {
                    manager.navigate(
                        to: .confirmPin(TapSignerConfirmPinArgs(from: args, newPin: newPin))
                    )
                }) {
                    Text("Continue")
                        .frame(maxWidth: .infinity)
                        .padding()
                        .background(newPin.count >= 6 ? Color.blue : Color.gray)
                        .foregroundColor(.white)
                        .cornerRadius(10)
                }
                .disabled(newPin.count < 6)
                .padding(.horizontal)

                Spacer()
            }
            .onAppear {
                newPin = ""
                isFocused = true
            }
            .onChange(of: isFocused) { _, _ in isFocused = true }
            .onChange(of: newPin) { _, pin in
                if pin.count > 32 {
                    newPin = String(pin.prefix(32))
                }
            }
        }
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }
}

#Preview {
    TapSignerContainer(
        route: .newPin(
            TapSignerNewPinArgs(
                tapSigner: tapSignerPreviewNew(preview: true),
                startingPin: "123456",
                chainCode: nil,
                action: .setup
            )
        )
    )
    .environment(AppManager.shared)
    .environment(AuthManager.shared)
}
