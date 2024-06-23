//
//  GradientButton.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

let minWidth: CGFloat = 125
let paddingVertical: CGFloat = 15
let paddingHorizontal: CGFloat = 25

struct GradientButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .frame(minWidth: minWidth)
            .padding(.horizontal, paddingHorizontal)
            .padding(.vertical, paddingVertical)
            .background(
                LinearGradient(
                    gradient: Gradient(colors: [
                        Color(red: 0.2, green: 0.4, blue: 1.0),
                        Color(red: 0.1, green: 0.5, blue: 1.0),
                    ]),
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
            )
            .foregroundColor(.white)
            .cornerRadius(10)
            .shadow(color: Color.black.opacity(0.3), radius: 5, x: 0, y: 2)
            .font(.headline)
            .scaleEffect(configuration.isPressed ? 0.95 : 1)
            .frame(minWidth: 100)
    }
}

struct GlassyButtonStyle: ButtonStyle {
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
                LinearGradient(
                    colors: [
                        Color(red: 0.0, green: 0.5, blue: 0.7),
                        Color(red: 0.0, green: 0.4, blue: 0.6),
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
