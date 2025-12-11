//
//  CommonButtonStyles.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

let minWidth: CGFloat = 125
let paddingVertical: CGFloat = 15
let paddingHorizontal: CGFloat = 25

struct GradientButtonStyle: ButtonStyle {
    var disabled = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(minWidth: minWidth)
            .padding(.horizontal, paddingHorizontal)
            .padding(.vertical, paddingVertical)
            .background(
                disabled ?
                    LinearGradient(
                        gradient: Gradient(colors: [
                            Color.gray.opacity(0.8),
                            Color.gray.opacity(0.7),
                        ]),
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    ) :
                    LinearGradient(
                        gradient: Gradient(colors: [
                            Color.btnGradientLight,
                            Color.btnGradientDark,
                        ]),
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
            )
            .foregroundColor(disabled ? Color.white.opacity(0.6) : Color.white)
            .cornerRadius(10)
            .shadow(color: Color.black.opacity(0.3), radius: 5, x: 0, y: 2)
            .font(.headline)
            .scaleEffect(configuration.isPressed ? 0.95 : 1)
            .frame(minWidth: 100)
    }
}

struct GlassyButtonStyle: ButtonStyle {
    var disabled = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(minWidth: minWidth)
            .padding(.horizontal, paddingHorizontal)
            .padding(.vertical, paddingVertical)
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(Color.white.opacity(0.2))
                    .overlay(
                        RoundedRectangle(cornerRadius: 10)
                            .stroke(Color.white.opacity(0.5), lineWidth: 1)
                    )
            )
            .shadow(color: Color.white.opacity(0.3), radius: 5, x: 0, y: 0)
            .foregroundStyle(
                disabled ?
                    LinearGradient(
                        colors: [
                            Color.gray,
                        ],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    ) :
                    LinearGradient(
                        colors: [
                            Color.btnGradientLight,
                            Color.btnGradientDark,
                        ],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
            )
            .font(.headline)
            .scaleEffect(configuration.isPressed ? 0.95 : 1)
            .animation(.easeInOut(duration: 0.2), value: configuration.isPressed)
    }
}
