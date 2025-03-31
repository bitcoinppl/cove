//
//  TapSignerConfirmPinView.swift
//  Cove
//
//  Created by Praveen Perera on 3/12/25.
//

import SwiftUI

struct TapSignerConfirmPinView: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let args: TapSignerConfirmPinArgs

    // private
    @State private var confirmPin: String = ""
    @State private var animateField: Bool = false
    @FocusState private var isFocused

    var chainCodeBytes: Data? {
        guard let chainCode = args.chainCode else { return nil }
        return hexDecode(hex: chainCode)
    }

    func checkPin() {
        if confirmPin != args.newPin {
            animateField.toggle()
            confirmPin = ""
            return
        }

        switch args.action {
        case .setup:
            setupTapSigner()
        case .change:
            changeTapSignerPin()
        }
    }

    func setupTapSigner() {
        let nfc = manager.getOrCreateNfc(args.tapSigner)

        Task {
            let response = await nfc.setupTapSigner(
                factoryPin: args.startingPin, newPin: args.newPin, chainCode: chainCodeBytes
            )
            await MainActor.run {
                switch response {
                case let .success(.complete(c)):
                    manager.resetRoute(to: .setupSuccess(args.tapSigner, c))
                case let .success(incomplete):
                    manager.resetRoute(to: .setupRetry(args.tapSigner, incomplete))
                case let .failure(error):
                    // failed to setup but we can continue
                    if let incomplete = nfc.lastResponse()?.setupResponse {
                        return manager.resetRoute(to: .setupRetry(args.tapSigner, incomplete))
                    }

                    // failed to setup and can't continue from a screen, send back to home and ask them to restart the process
                    Log.error("Failed to setup TapSigner: \(error)")
                    app.sheetState = .none
                    app.alertState = .init(.tapSignerSetupFailed(error.describe))
                }
            }
        }
    }

    func changeTapSignerPin() {
        let nfc = manager.getOrCreateNfc(args.tapSigner)
        Task {
            let response = await nfc.changePin(
                currentPin: args.startingPin, newPin: args.newPin
            )
            switch response {
            case .success:
                app.alertState = .init(
                    .general(title: "PIN Changed", message: "Your TAPSIGNER PIN was changed succesfully!")
                )
            case let .failure(error):
                if error.isAuthError {
                    app.alertState = .init(.tapSignerInvalidAuth)
                    return
                }
                app.alertState = .init(.general(title: "Error", message: error.describe))
            }
        }
    }

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
                .keyframeAnimator(
                    initialValue: CGFloat.zero,
                    trigger: animateField,
                    content: { content, value in
                        content
                            .offset(x: value)
                    },
                    keyframes: { _ in
                        KeyframeTrack {
                            CubicKeyframe(30, duration: 0.07)
                            CubicKeyframe(-30, duration: 0.07)
                            CubicKeyframe(20, duration: 0.07)
                            CubicKeyframe(-20, duration: 0.07)
                            CubicKeyframe(10, duration: 0.07)
                            CubicKeyframe(-10, duration: 0.07)
                            CubicKeyframe(0, duration: 0.07)
                        }
                    }
                )
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
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) {
                        checkPin()
                    }
                }

                if pin.count > 6, old.count < 6 {
                    confirmPin = old
                    return
                }

                if pin.count > 6 {
                    confirmPin = String(args.startingPin.prefix(6))
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
            TapSignerConfirmPinArgs(
                tapSigner: tapSignerPreviewNew(preview: true),
                startingPin: "123456",
                newPin: "222222",
                chainCode: nil,
                action: .setup
            )
        )
    )
    .environment(AppManager.shared)
}
