//
//  LockView.swift
//  Cove
//
//  Created by Praveen Perera on 12/10/24.
//

import LocalAuthentication
import SwiftUI

extension UIApplication {
    func endEditing() {
        sendAction(
            #selector(UIResponder.resignFirstResponder),
            to: nil, from: nil, for: nil
        )
    }
}

private enum Screen {
    case biometric, pin
}

enum LockState: Equatable {
    case locked, unlocked
}

struct LockView<Content: View>: View {
    @Environment(AppManager.self) var app
    @Environment(AuthManager.self) var auth

    /// Args: Lock Properties
    var lockType: AuthType
    var isPinCorrect: (String) -> Bool
    var showPin: Bool
    var bioMetricUnlockMessage: String

    /// default calllbacks on success and failure
    var onUnlock: (String) -> Void
    var onWrongPin: (String) -> Void

    @ViewBuilder var content: Content

    // lock state
    private let lockStateBinding: Binding<LockState>?
    @State private var innerLockState: LockState = .locked

    /// back button
    private var backEnabled: Bool
    var _backAction: (() -> Void)?

    /// View Properties
    @State private var animateField: Bool
    @State private var screen: Screen = .biometric

    /// private consts
    private let pinLength: Int

    /// Scene Phase
    @Environment(\.scenePhase) private var phase

    init(
        lockType: AuthType,
        isPinCorrect: @escaping (String) -> Bool,
        showPin: Bool = false,
        lockState: Binding<LockState>? = nil,
        bioMetricUnlockMessage: String = "Unlock your wallet",
        onUnlock: @escaping (String) -> Void = { _ in },
        onWrongPin: @escaping (String) -> Void = { _ in },
        backAction: (() -> Void)? = nil,
        @ViewBuilder content: () -> Content
    ) {
        self.lockType = lockType

        innerLockState = .locked
        lockStateBinding = lockState

        self.isPinCorrect = isPinCorrect
        self.showPin = showPin
        self.bioMetricUnlockMessage = bioMetricUnlockMessage
        self.onUnlock = onUnlock
        self.onWrongPin = onWrongPin
        self.content = content()

        // back
        let backEnabled =
            if backAction != nil { true } else {
                lockType == .both && isBiometricAvailable
            }

        self.backEnabled = backEnabled
        _backAction = backAction

        // private
        animateField = false
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

    private var lockState: Binding<LockState> {
        lockStateBinding
            ?? Binding(
                get: { innerLockState },
                set: { innerLockState = $0 }
            )
    }

    var body: some View {
        GeometryReader {
            let size = $0.size

            content
                .frame(width: size.width, height: size.height)

            if lockState.wrappedValue == .locked {
                ZStack {
                    Rectangle()
                        .fill(.black)
                        .ignoresSafeArea()

                    switch (screen, lockType, isBiometricAvailable) {
                    case (_, .biometric, false):
                        PermissionsNeeded
                    case (_, .biometric, true):
                        BiometricView
                    case (.biometric, .both, true):
                        BiometricView
                    case (_, .pin, _):
                        numberPadPinView
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
        .onChange(of: lockState.wrappedValue) { _, state in
            if state == .locked { tryUnlockingView() }
        }
        .onChange(of: phase) { old, phase in
            if old == .inactive, phase == .background, lockType == .both {
                screen = .biometric
            }

            if old == .background, phase == .inactive, lockState.wrappedValue == .locked {
                tryUnlockingView()
            }
        }
        .onAppear {
            tryUnlockingView()
        }
    }

    var numberPadPinView: NumberPadPinView {
        NumberPadPinView(
            lockState: lockState,
            isPinCorrect: isPinCorrect,
            showPin: showPin,
            pinLength: pinLength,
            backAction: backEnabled ? backAction : nil,
            onUnlock: onUnlock,
            onWrongPin: onWrongPin
        )
    }

    var PermissionsNeeded: some View {
        VStack(spacing: 20) {
            Text(
                "Cove needs permissions to FaceID to unlock your wallet. Please open settings and enable FaceID."
            )
            .font(.callout)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 50)

            Button("Open Settings") {
                let url = URL(string: UIApplication.openSettingsURLString)!
                UIApplication.shared.open(url)
            }
        }
    }

    var BiometricView: some View {
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
                Button(action: { screen = .pin }) {
                    Text("Enter Pin")
                        .frame(width: 100, height: 40)
                        .background(
                            .ultraThinMaterial,
                            in: .rect(cornerRadius: 10)
                        )
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .foregroundStyle(.white)
            }
        }
        .frame(maxHeight: .infinity)
    }

    private func bioMetricUnlock() async throws -> Bool {
        // Lock Context
        let context = LAContext()

        return try await context.evaluatePolicy(
            .deviceOwnerAuthentication,
            localizedReason: bioMetricUnlockMessage
        )
    }

    private func tryUnlockingView() {
        guard lockType == .biometric || lockType == .both else { return }
        guard !auth.isUsingBiometrics else { return }
        guard isBiometricAvailable else { return }
        guard lockState.wrappedValue == .locked else { return }

        // Checking and Unlocking View
        Task {
            // Requesting Biometric Unlock
            auth.isUsingBiometrics = true

            if await (try? bioMetricUnlock()) ?? false {
                await MainActor.run {
                    withAnimation(.snappy, completionCriteria: .logicallyComplete) {
                        lockState.wrappedValue = .unlocked
                    } completion: {
                        auth.isUsingBiometrics = false
                        onUnlock("")
                    }
                }
            } else {
                await MainActor.run { auth.isUsingBiometrics = false }
            }
        }
    }
}

private var isBiometricAvailable: Bool {
    // Lock Context
    let context = LAContext()
    return context.canEvaluatePolicy(.deviceOwnerAuthentication, error: nil)
}

#Preview("normal") {
    LockView(lockType: .both, isPinCorrect: { $0 == "111111" }) {
        VStack {
            Text("Hello World")
        }
    }
    .environment(AppManager.shared)
}

#Preview("need permissions") {
    LockView(lockType: .biometric, isPinCorrect: { $0 == "111111" }) {
        VStack {
            Text("Hello World")
        }
    }
    .environment(AppManager.shared)
}

#Preview("with navigation") {
    NavigationStack {
        LockView(lockType: .pin, isPinCorrect: { $0 == "111111" }) {
            VStack {
                Text("Hello World")
            }
        }
        .toolbar {
            ToolbarItem(placement: .navigationBarLeading) {
                Button("Cancel") {
                    ()
                }
            }

            ToolbarItem(placement: .principal) {
                Text("Lock").foregroundStyle(.white)
            }
        }
    }
    .environment(AppManager.shared)
}
