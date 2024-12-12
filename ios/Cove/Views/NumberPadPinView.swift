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
    var title: String = "Enter Pin"
    @Binding var isUnlocked: Bool
    
    let isPinCorrect: (String) -> Bool
    var pinLength: Int = 6

    /// default calllbacks on success and failure
    var onUnlock: (String) -> Void = { _ in }
    var onWrongPin: (String) -> Void = { _ in }

    /// private view properties
    @State private var pin: String = ""
    @State private var animateField: Bool = false
    
    private var isBiometricAvailable: Bool {
        /// Lock Context
        let context = LAContext()
        return context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
    }

    var body: some View {
        VStack(spacing: 15) {
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
        @State var isUnlocked = false
        
        var body: some View {
            NumberPadPinView(
                isUnlocked: $isUnlocked,
                isPinCorrect: { $0 == "000000" },
                pinLength: 6
            )
        }
    }
                
    return Container()
}
