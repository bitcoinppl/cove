//
//  LockView.swift
//  Cove
//
//  Created by Praveen Perera on 12/10/24.
//

import LocalAuthentication
import SwiftUI

private enum Screen {
    case biometric, pin
}

struct LockView<Content: View>: View {
    /// Args: Lock Properties
    var lockType: AuthType
    var isPinCorrect: (String) -> Bool
    var isEnabled: Bool
    var bioMetricUnlockMessage: String

    /// default calllbacks on success and failure
    var onUnlock: (String) -> Void
    var onWrongPin: (String) -> Void

    @ViewBuilder var content: Content

    /// back button
    private var backEnabled: Bool
    var _backAction: (() -> Void)?

    /// View Properties
    @State private var animateField: Bool
    @State private var isUnlocked: Bool
    @State private var screen: Screen = .biometric

    /// private consts
    private let pinLength: Int

    /// Scene Phase
    @Environment(\.scenePhase) private var phase

    init(
        lockType: AuthType,
        isPinCorrect: @escaping (String) -> Bool,
        isEnabled: Bool = true,
        bioMetricUnlockMessage: String = "Unlock your wallet",
        onUnlock: @escaping (String) -> Void = { _ in },
        onWrongPin: @escaping (String) -> Void = { _ in },
        backAction: (() -> Void)? = nil,
        @ViewBuilder content: () -> Content
    ) {
        self.lockType = lockType
        self.isPinCorrect = isPinCorrect
        self.isEnabled = isEnabled
        self.bioMetricUnlockMessage = bioMetricUnlockMessage
        self.onUnlock = onUnlock
        self.onWrongPin = onWrongPin
        self.content = content()

        // back
        let backEnabled = if backAction != nil { true } else {
            lockType == .both && isBiometricAvailable
        }

        self.backEnabled = backEnabled
        _backAction = backAction

        // private
        animateField = false
        isUnlocked = false
        pinLength = 6
    }

    var backAction: () -> Void {
        if let _backAction { return _backAction }

        if backEnabled {
            return {
                withAnimation {
                    screen = .biometric
                }
            }
        } else {
            return {}
        }
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

                    switch (screen, lockType, isBiometricAvailable) {
                    case (_, .biometric, true):
                        PinOrBioMetric
                    case (_, .biometric, false):
                        PermissionsNeeded
                    case (_, .pin, _):
                        numberPadPinView
                    case (.biometric, .both, true):
                        PinOrBioMetric
                    case (.biometric, .both, false):
                        numberPadPinView
                    case (.pin, .both, _):
                        numberPadPinView
                    case (_, .none, _):
                        let _ = Log.error("inalid lock type none for screen")
                        EmptyView()
                    }
                }
                .environment(\.colorScheme, .dark)
                .transition(.offset(y: size.height + 100))
            }
        }
        .onChange(of: isEnabled, initial: true) { _, newValue in
            if newValue { tryUnlockingView() }
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
    var PermissionsNeeded: some View {
        VStack(spacing: 20) {
            Text("Enable biometric authentication in Settings to unlock the view.")
                .font(.callout)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 50)

            Button("Open Settings") {
                let url = URL(string: UIApplication.openSettingsURLString)!
                UIApplication.shared.open(url)
            }
        }
    }

    @ViewBuilder
    var PinOrBioMetric: some View {
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
}

private var isBiometricAvailable: Bool {
    /// Lock Context
    let context = LAContext()
    return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
}

#Preview("normal") {
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

#Preview("need permissions") {
    LockView(
        lockType: .biometric,
        isPinCorrect: { $0 == "111111" },
        isEnabled: true
    ) {
        VStack {
            Text("Hello World")
        }
    }
}
