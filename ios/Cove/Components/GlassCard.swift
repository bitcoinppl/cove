//
//  GlassCard.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

struct GlassCard<Content: View>: View {
    var colors: [Color] = [.orange, Color.red.opacity(0.6)]
    var shadowRadius: CGFloat = 0
    var shadowColor: Color = .gray

    @ViewBuilder var content: Content

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 10)
                .fill(LinearGradient(colors: colors, startPoint: .topLeading, endPoint: .bottomTrailing))

            // Glass effect layer
            RoundedRectangle(cornerRadius: 10)
                .fill(.ultraThinMaterial)
                .shadow(color: shadowColor, radius: shadowRadius)

            // Border layer
            RoundedRectangle(cornerRadius: 10)
                .stroke(.background.opacity(0.2), lineWidth: 1)

            // content
            content
        }

    }

}

#Preview {
    VStack(spacing: 20) {
        GlassCard {
            VStack {
                Text("Glass Card")
                    .font(.title)
                    .foregroundColor(.white)
                Text("Customonizable")
                    .foregroundColor(.white.opacity(0.7))
            }
        }
        .frame(width: 300, height: 200)

        GlassCard(colors: [.blue, Color.blue.opacity(0.8)], shadowRadius: 10) {
            VStack {
                Text("Glass Card")
                    .font(.title)
                    .foregroundColor(.white)
                Text("Customonizable")
                    .foregroundColor(.white.opacity(0.7))
            }
        }
        .frame(width: 300, height: 200)

        GlassCard(colors: [Color.purple.opacity(0.9), Color.purple.opacity(0.7), Color.purple.opacity(0.6)], shadowRadius: 10) {
            VStack {
                Text("Glass Card")
                    .font(.title)
                    .foregroundColor(.white)
                Text("Customonizable")
                    .foregroundColor(.white.opacity(0.7))
            }
        }
        .frame(width: 300, height: 200)
    }
}
