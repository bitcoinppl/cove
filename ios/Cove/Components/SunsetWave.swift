//
//  OrangeBackgroundGradientView.swift
//  Cove
//
//  Created by Praveen Perera on 6/21/24.
//

import SwiftUI

struct SunsetWave<Content: View>: View {
    @ViewBuilder
    let content: Content

    var body: some View {
        ZStack {
            // Warm sunset background gradient
            LinearGradient(gradient: Gradient(colors: [
                Color(red: 0.98, green: 0.4, blue: 0.3), // Soft red
                Color(red: 0.98, green: 0.6, blue: 0.3), // Soft orange
                Color(red: 0.95, green: 0.8, blue: 0.6) // Soft light orange
            ]), startPoint: .topLeading, endPoint: .bottomTrailing)

            // Overlapping waves
            WaveView(color1: Color(red: 0.8, green: 0.3, blue: 0.2), color2: Color(red: 0.9, green: 0.5, blue: 0.3), frequency: 0.5, amplitude: 100, phase: 0)
                .opacity(0.4)

            WaveView(color1: Color(red: 0.9, green: 0.4, blue: 0.2), color2: Color(red: 1.0, green: 0.6, blue: 0.4), frequency: 0.6, amplitude: 130, phase: 0.5)
                .opacity(0.3)

            WaveView(color1: Color(red: 1.0, green: 0.5, blue: 0.3), color2: Color(red: 1.0, green: 0.7, blue: 0.5), frequency: 0.7, amplitude: 160, phase: 1)
                .opacity(0.2)

            // content
            content
        }
        .ignoresSafeArea()
    }
}

struct WaveView: View {
    let color1: Color
    let color2: Color
    let frequency: Double
    let amplitude: Double
    let phase: Double

    var body: some View {
        GeometryReader { geometry in
            Path { path in
                let width = geometry.size.width
                let height = geometry.size.height
                let midHeight = height / 2

                path.move(to: CGPoint(x: 0, y: midHeight))

                for x in stride(from: 0, through: width, by: 1) {
                    let relativeX = x / width
                    let y = sin(relativeX * .pi * frequency * 2 + phase) * amplitude + midHeight
                    path.addLine(to: CGPoint(x: x, y: y))
                }

                path.addLine(to: CGPoint(x: width, y: height))
                path.addLine(to: CGPoint(x: 0, y: height))
                path.closeSubpath()
            }
            .fill(LinearGradient(gradient: Gradient(colors: [color1, color2]), startPoint: .top, endPoint: .bottom))
        }
    }
}

#Preview {
    SunsetWave {}
}
