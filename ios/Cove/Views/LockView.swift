//
//  LockView.swift
//  Cove
//
//  Created by Praveen Perera on 12/10/24.
//

import LocalAuthentication
import SwiftUI

struct LockView<Content: View>: View {
    /// Args: Lock Properties
    var lockType: LockType
    var isPinCorrect: (String) -> Bool
    var isEnabled: Bool
    var lockWhenBackground: Bool = true
    var bioMetricUnlockMessage: String = "Unlock your wallet"
    @ViewBuilder var content: Content

    /// View Properties
    @State private var pin: String = ""
    @State private var animateField: Bool = false
    @State private var isUnlocked: Bool = false
    @State private var noBiometricAccess: Bool = false

    /// private consts
    private let pinLength = 6

    /// Scene Phase
    @Environment(\.scenePhase) private var phase

    var body: some View {
        GeometryReader {
            let size = $0.size

            content
                .frame(width: size.width, height: size.height)

            if isEnabled, !isUnlocked {
                ZStack {
                    Rectangle()
                        .fill(.black)
                        .ignoresSafeArea()

                    if (lockType == .both && !noBiometricAccess) || lockType == .biometric {
                        Group {
                            if noBiometricAccess {
                                Text("Enable biometric authentication in Settings to unlock the view.")
                                    .font(.callout)
                                    .multilineTextAlignment(.center)
                                    .padding(50)
                            } else {
                                /// Bio Metric / Pin Unlock
                                VStack(spacing: 12) {
                                    VStack(spacing: 6) {
                                        Image(systemName: "faceid")
                                            .font(.largeTitle)

                                        Text("Tap to Unlock")
                                            .font(.caption2)
                                            .foregroundStyle(.gray)
                                    }
                                    .frame(width: 100, height: 100)
                                    .background(.ultraThinMaterial, in: .rect(cornerRadius: 10))
                                    .contentShape(.rect)
                                    .onTapGesture {
                                        unlockView()
                                    }

                                    if lockType == .both {
                                        Text("Enter Pin")
                                            .frame(width: 100, height: 40)
                                            .background(.ultraThinMaterial,
                                                        in: .rect(cornerRadius: 10))
                                            .contentShape(.rect)
                                            .onTapGesture {
                                                noBiometricAccess = true
                                            }
                                    }
                                }
                            }
                        }
                    } else {
                        /// Custom Number Pad to type View Lock Pin
                        NumberPadPinView()
                    }
                }
                .environment(\.colorScheme, .dark)
                .transition(.offset(y: size.height + 100))
            }
        }
        .onChange(of: isEnabled, initial: true) { _, newValue in
            if newValue {
                unlockView()
            }
        }
        /// Locking When App Goes Background
        .onChange(of: phase) { _, newValue in
            if newValue != .active, lockWhenBackground {
                isUnlocked = false
                pin = ""
            }
        }
    }

    private func bioMetricUnlock() async throws -> Bool {
        /// Lock Context
        let context = LAContext()

        return try await context.evaluatePolicy(
            .deviceOwnerAuthenticationWithBiometrics,
            localizedReason: bioMetricUnlockMessage
        )
    }

    private func unlockView() {
        /// Checking and Unlocking View
        Task {
            guard isBiometricAvailable, lockType != .pin else {
                /// No Bio Metric Permission || Lock Type Must be Set as Keypad
                /// Updating Biometric Status
                await MainActor.run { noBiometricAccess = !isBiometricAvailable }
                return
            }

            /// Requesting Biometric Unlock
            if await (try? bioMetricUnlock()) ?? false {
                await MainActor.run {
                    withAnimation(
                        .snappy,
                        completionCriteria: .logicallyComplete
                    ) {
                        isUnlocked = true
                    } completion: { pin = "" }
                }
            }
        }
    }

    private var isBiometricAvailable: Bool {
        /// Lock Context
        let context = LAContext()
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
    }

    /// Numberpad Pin View
    @ViewBuilder
    private func NumberPadPinView() -> some View {
        VStack(spacing: 15) {
            Text("Enter Pin")
                .font(.title.bold())
                .frame(maxWidth: .infinity)
                .overlay(alignment: .leading) {
                    /// Back button only for Both Lock Type
                    if lockType == .both, isBiometricAvailable {
                        Button(action: {
                            pin = ""
                            noBiometricAccess = false
                        }, label: {
                            Image(systemName: "arrow.left")
                                .font(.title3)
                                .contentShape(.rect)
                        })
                        .tint(.white)
                        .padding(.leading)
                    }
                }

            /// Adding Wiggling Animation for Wrong Password With Keyframe Animator
            HStack(spacing: 10) {
                ForEach(0 ..< pinLength, id: \.self) { index in
                    RoundedRectangle(cornerRadius: 10)
                        .frame(width: 40, height: 45)
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
                })
                .frame(maxHeight: .infinity, alignment: .bottom)
            }
            .onChange(of: pin) { _, newValue in
                if newValue.count == pinLength {
                    /// Validate Pin
                    if isPinCorrect(pin) {
                        withAnimation(.snappy, completionCriteria: .logicallyComplete) {
                            isUnlocked = true
                        } completion: {
                            pin = ""
                            noBiometricAccess = !isBiometricAvailable
                        }
                    } else {
                        pin = ""
                        animateField.toggle()
                    }
                }
            }
        }
        .padding()
        .environment(\.colorScheme, .dark)
    }
}

#Preview {
    LockView(lockType: .both, isPinCorrect: { $0 == "111111" }, isEnabled: true) {
        VStack {
            Text("Hello World")
        }
    }
}
