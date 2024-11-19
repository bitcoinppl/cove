//
//  FixedGlassCard.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

struct FixedGlassCard<Content: View>: View {
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
            RoundedRectangle(cornerRadius: 10)
                .fill(.ultraThinMaterial)
                .shadow(color: .orange.opacity(0.3), radius: 20, x: 0, y: 10)

            // Border layer
            RoundedRectangle(cornerRadius: 10)
                .stroke(.background.opacity(0.2), lineWidth: 1)

            // content
            content
        }
    }
}

#Preview {
    FixedGlassCard {
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
