//
//  NumberPadPinView.swift
//  Cove
//
//  Created by Praveen Perera on 12/11/24.
//
import LocalAuthentication
import SwiftUI

struct NumberPadPinView: View {
    /// args
    var title: String
    @Binding var lockState: LockState

    let isPinCorrect: (String) -> Bool
    var pinLength: Int

    // back button
    private var backEnabled: Bool
    var backAction: () -> Void

    /// default calllbacks on success and failure
    var onUnlock: (String) -> Void
    var onWrongPin: (String) -> Void

    /// private view properties
    @State private var pin: String
    @State private var animateField: Bool

    public init(
        title: String = "Enter Pin",
        lockState: Binding<LockState> = .constant(.unlocked),
        isPinCorrect: @escaping (String) -> Bool,
        pinLength: Int = 6,
        backAction: (() -> Void)? = nil,
        onUnlock: @escaping (String) -> Void = { _ in },
        onWrongPin: @escaping (String) -> Void = { _ in }
    ) {
        self.title = title
        _lockState = lockState
        self.isPinCorrect = isPinCorrect
        self.pinLength = pinLength
        backEnabled = backAction != nil
        self.backAction = backAction ?? {}
        self.onUnlock = onUnlock
        self.onWrongPin = onWrongPin

        pin = ""
        animateField = false
    }

    private var isBiometricAvailable: Bool {
        /// Lock Context
        let context = LAContext()
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
    }

    var body: some View {
        VStack(spacing: 15) {
            if backEnabled {
                HStack {
                    Spacer()
                    Button(action: backAction) {
                        Text("Cancel")
                    }
                    .font(.headline.bold())
                    .foregroundStyle(.white)
                }
                .padding(.bottom, 10)
            }

            Text(title)
                .font(.title.bold())
                .frame(maxWidth: .infinity)
                .foregroundStyle(.white)

            /// Adding Wiggling Animation for Wrong Password With Keyframe Animator
            HStack(spacing: 10) {
                ForEach(0 ..< pinLength, id: \.self) { index in
                    RoundedRectangle(cornerRadius: 10)
                        .frame(width: 40, height: 45)
                        .foregroundStyle(.white)
                        /// Showing Pin at each box with the help of Index
                        .overlay {
                            /// Safe Check
                            if pin.count > index {
                                let index = pin.index(pin.startIndex, offsetBy: index)
                                let string = String(pin[index])

                                Text(string)
                                    .font(.title.bold())
                                    .foregroundStyle(.black)
                            }
                        }
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
            /// run onEnd call back after keyframe animation
            .onChange(of: animateField) { _, _ in
                let pin = pin
                self.pin = ""

                let totalDuration = 7 * 0.07
                DispatchQueue.main.asyncAfter(deadline: .now() + totalDuration) {
                    onWrongPin(pin)
                }
            }
            .padding(.top, 15)
            .frame(maxHeight: .infinity)

            /// Custom Number Pad
            GeometryReader { _ in
                LazyVGrid(columns: Array(repeating: GridItem(), count: 3), content: {
                    ForEach(1 ... 9, id: \.self) { number in
                        Button(action: {
                            guard pin.count < pinLength else { return }
                            pin.append(String(number))
                        }, label: {
                            Text(String(number))
                                .font(.title)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 20)
                                .contentShape(.rect)
                        })
                        .tint(.white)
                    }

                    // take up space
                    Button(action: {}) {}

                    Button(action: {
                        guard pin.count < pinLength else { return }
                        pin.append("0")
                    }, label: {
                        Text("0")
                            .font(.title)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 20)
                            .contentShape(.rect)
                    })
                    .tint(.white)

                    /// 0 and Back Button
                    Button(action: {
                        if !pin.isEmpty { pin.removeLast() }
                    }, label: {
                        Image(systemName: "delete.backward")
                            .font(.title)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 20)
                            .contentShape(.rect)
                    })
                    .tint(.white)
                })
                .frame(maxHeight: .infinity, alignment: .bottom)
            }
            .onChange(of: pin) { _, newValue in
                if newValue.count == pinLength {
                    /// Validate Pin
                    if isPinCorrect(pin) {
                        withAnimation(.snappy, completionCriteria: .logicallyComplete) {
                            lockState = .unlocked
                        } completion: {
                            onUnlock(pin)
                            pin = ""
                        }
                    } else {
                        animateField.toggle()
                    }
                }
            }
        }
        .padding()
        .background(.midnightBlue)
    }
}

#Preview {
    struct Container: View {
        @State var pin = ""
        @State var lockState: LockState = .locked

        var body: some View {
            NumberPadPinView(
                lockState: $lockState,
                isPinCorrect: { $0 == "000000" },
                pinLength: 6
            )
        }
    }

    return Container()
}
