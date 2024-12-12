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
    var lockWhenBackground: Bool
    var bioMetricUnlockMessage: String

    /// default calllbacks on success and failure
    var onUnlock: (String) -> Void
    var onWrongPin: (String) -> Void

    @ViewBuilder var content: Content

    /// back button
    private var backEnabled: Bool
    var backAction: () -> Void

    /// View Properties
    @State private var animateField: Bool
    @State private var isUnlocked: Bool
    @State private var noBiometricAccess: Bool

    /// private consts
    private let pinLength: Int

    /// Scene Phase
    @Environment(\.scenePhase) private var phase

    init(
        lockType: AuthType,
        isPinCorrect: @escaping (String) -> Bool,
        isEnabled: Bool = true,
        lockWhenBackground: Bool = true,
        bioMetricUnlockMessage: String = "Unlock your wallet",
        onUnlock: @escaping (String) -> Void = { _ in },
        onWrongPin: @escaping (String) -> Void = { _ in },
        backAction: (() -> Void)? = nil,
        @ViewBuilder content: () -> Content
    ) {
        self.lockType = lockType
        self.isPinCorrect = isPinCorrect
        self.isEnabled = isEnabled
        self.lockWhenBackground = lockWhenBackground
        self.bioMetricUnlockMessage = bioMetricUnlockMessage
        self.onUnlock = onUnlock
        self.onWrongPin = onWrongPin
        self.content = content()

        // back
        self.backEnabled = backAction != nil
        self.backAction = backAction ?? {}

        // private
        self.animateField = false
        self.isUnlocked = false
        self.noBiometricAccess = false
        self.pinLength = 6
    }

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
                        PinOrBioMetric
                    } else {
                        numberPadPinView
                    }
                }
                .environment(\.colorScheme, .dark)
                .transition(.offset(y: size.height + 100))
            }
        }
        .onChange(of: isEnabled, initial: true) { _, newValue in
            if newValue { tryUnlockingView() }
        }
        /// Locking When App Goes Background
        .onChange(of: phase) { _, newValue in
            if newValue != .active, lockWhenBackground {
                isUnlocked = false
            }
        }
    }

    var numberPadPinView: NumberPadPinView {
        NumberPadPinView(
            isUnlocked: $isUnlocked,
            isPinCorrect: isPinCorrect,
            pinLength: pinLength,
            backAction: backEnabled ? backAction : nil,
            onUnlock: onUnlock,
            onWrongPin: onWrongPin
        )
    }

    @ViewBuilder
    var PinOrBioMetric: some View {
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
                    .onTapGesture { tryUnlockingView() }

                    if lockType == .both {
                        Text("Enter Pin")
                            .frame(width: 100, height: 40)
                            .background(
                                .ultraThinMaterial,
                                in: .rect(cornerRadius: 10)
                            )
                            .contentShape(.rect)
                            .onTapGesture { noBiometricAccess = true }
                    }
                }
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

    private func tryUnlockingView() {
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
                    withAnimation(.snappy, completionCriteria: .logicallyComplete) {
                        isUnlocked = true
                    } completion: {
                        onUnlock("")
                    }
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
    LockView(
        lockType: .both,
        isPinCorrect: { $0 == "111111" },
        isEnabled: true
    ) {
        VStack {
            Text("Hello World")
        }
    }
}
