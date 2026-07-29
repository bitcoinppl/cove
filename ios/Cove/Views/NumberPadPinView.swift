//
//  NumberPadPinView.swift
//  Cove
//
//  Created by Praveen Perera on 12/11/24.
//
import SwiftUI

struct NumberPadPinView: View {
    /// args
    var title: String
    @Binding var lockState: LockState

    let isPinCorrect: (String) -> Bool
    var showPin: Bool
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
        showPin: Bool = false,
        pinLength: Int = 6,
        backAction: (() -> Void)? = nil,
        onUnlock: @escaping (String) -> Void = { _ in },
        onWrongPin: @escaping (String) -> Void = { _ in }
    ) {
        self.title = title
        _lockState = lockState
        self.isPinCorrect = isPinCorrect
        self.showPin = showPin
        self.pinLength = pinLength
        backEnabled = backAction != nil
        self.backAction = backAction ?? {}
        self.onUnlock = onUnlock
        self.onWrongPin = onWrongPin

        pin = ""
        animateField = false
    }

    var body: some View {
        VStack(spacing: 16) {
            NumberPadPinTitle(title: title)

            Spacer()

            PinEntryDisplay(
                pin: $pin,
                animateField: $animateField,
                showPin: showPin,
                pinLength: pinLength,
                onWrongPin: onWrongPin
            )

            Spacer()
            Spacer()

            PinNumberPad(
                pin: $pin,
                animateField: $animateField,
                lockState: $lockState,
                pinLength: pinLength,
                backEnabled: backEnabled,
                backAction: backAction,
                isPinCorrect: isPinCorrect,
                onUnlock: onUnlock
            )
        }
        .padding()
        .background(.midnightBlue)
    }
}

private struct NumberPadPinTitle: View {
    let title: String

    var body: some View {
        Text(title)
            .font(.title.bold())
            .frame(maxWidth: .infinity)
            .foregroundStyle(.white)
            .padding(.vertical)
    }
}

private struct PinEntryDisplay: View {
    @Binding var pin: String
    @Binding var animateField: Bool
    let showPin: Bool
    let pinLength: Int
    let onWrongPin: (String) -> Void

    var body: some View {
        HStack(spacing: 10) {
            ForEach(0 ..< pinLength, id: \.self) { index in
                PinDigitBox(pin: pin, index: index, showPin: showPin)
            }
        }
        .keyframeAnimator(
            initialValue: CGFloat.zero,
            trigger: animateField,
            content: { content, value in
                content.offset(x: value)
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
        .onChange(of: animateField) { _, _ in
            let wrongPin = pin
            pin = ""

            let totalDuration = 7 * 0.07
            DispatchQueue.main.asyncAfter(deadline: .now() + totalDuration) {
                onWrongPin(wrongPin)
            }
        }
        .padding(.top, 15)
    }
}

private struct PinDigitBox: View {
    let pin: String
    let index: Int
    let showPin: Bool

    var body: some View {
        RoundedRectangle(cornerRadius: 10)
            .frame(width: 40, height: 45)
            .foregroundStyle(.white)
            .overlay {
                if pin.count > index {
                    let pinIndex = pin.index(pin.startIndex, offsetBy: index)
                    let string = showPin ? String(pin[pinIndex]) : "●"

                    Text(string)
                        .font(showPin ? .title : .body)
                        .fontWeight(.bold)
                        .foregroundStyle(.black)
                }
            }
    }
}

private struct PinNumberPad: View {
    @Binding var pin: String
    @Binding var animateField: Bool
    @Binding var lockState: LockState
    let pinLength: Int
    let backEnabled: Bool
    let backAction: () -> Void
    let isPinCorrect: (String) -> Bool
    let onUnlock: (String) -> Void

    var body: some View {
        LazyVGrid(columns: Array(repeating: GridItem(), count: 3)) {
            ForEach(1 ... 9, id: \.self) { number in
                PinNumberButton(number: number, pin: $pin, pinLength: pinLength)
            }

            PinCancelButton(isEnabled: backEnabled, action: backAction)
            PinNumberButton(number: 0, pin: $pin, pinLength: pinLength)
            PinDeleteButton(pin: $pin)
        }
        .padding(.vertical)
        .onChange(of: pin) { _, newValue in
            validate(newValue)
        }
    }

    private func validate(_ newValue: String) {
        guard newValue.count == pinLength else { return }

        if isPinCorrect(newValue) {
            withAnimation(.snappy, completionCriteria: .logicallyComplete) {
                lockState = .unlocked
            } completion: {
                onUnlock(newValue)
                pin = ""
            }
        } else {
            animateField.toggle()
        }
    }
}

private struct PinNumberButton: View {
    let number: Int
    @Binding var pin: String
    let pinLength: Int

    var body: some View {
        Button {
            guard pin.count < pinLength else { return }

            pin.append(String(number))
        } label: {
            Text(String(number))
                .font(.title)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 20)
                .contentShape(.rect)
        }
        .tint(.white)
    }
}

private struct PinCancelButton: View {
    let isEnabled: Bool
    let action: () -> Void

    var body: some View {
        if isEnabled {
            Button("Cancel", action: action)
                .foregroundStyle(.white)
        } else {
            Button(action: {}) {}
        }
    }
}

private struct PinDeleteButton: View {
    @Binding var pin: String

    var body: some View {
        Button {
            guard !pin.isEmpty else { return }

            pin.removeLast()
        } label: {
            Image(systemName: "delete.backward")
                .font(.title)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 20)
                .contentShape(.rect)
        }
        .tint(.white)
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

#Preview("hidden pin") {
    struct Container: View {
        @State var pin = ""
        @State var lockState: LockState = .locked

        var body: some View {
            NumberPadPinView(
                lockState: $lockState,
                isPinCorrect: { $0 == "000000" },
                showPin: false,
                pinLength: 6
            )
        }
    }

    return Container()
}
