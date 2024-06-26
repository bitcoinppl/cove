//
//  GlassCard.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

struct GlassCard<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        ZStack {
            Circle()
                .fill(Color.orange)
                .blur(radius: 30)
                .offset(x: -50, y: -5)
            Circle()
                .fill(Color.red.opacity(0.5))
                .blur(radius: 30)
                .offset(x: 50, y: 10)

            // Glass effect layer
            RoundedRectangle(cornerRadius: 20)
                .fill(.ultraThinMaterial)
                .shadow(color: .orange.opacity(0.3), radius: 20, x: 0, y: 10)

            // Border layer
            RoundedRectangle(cornerRadius: 20)
                .stroke(.white.opacity(0.3), lineWidth: 1)

            // content
            content
        }
        .enableInjection()
    }

    #if DEBUG
    @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    GlassCard {
        VStack {
            Text("Glass Card")
                .font(.title)
                .foregroundColor(.white)
            Text("With warm orange glow")
                .foregroundColor(.white.opacity(0.7))
        }
    }
    .frame(width: 300, height: 200)
}
