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
    var lockType: AuthType
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
                        NumberPadPinView(
                            pin: $pin,
                            isUnlocked: $isUnlocked,
                            noBiometricAccess: $noBiometricAccess,
                            isPinCorrect: isPinCorrect,
                            lockType: lockType,
                            pinLength: pinLength
                        )
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
}

#Preview {
    LockView(lockType: .both, isPinCorrect: { $0 == "111111" }, isEnabled: true) {
        VStack {
            Text("Hello World")
        }
    }
}
