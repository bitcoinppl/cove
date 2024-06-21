//
//  OrangeBackgroundGradientView.swift
//  Cove
//
//  Created by Praveen Perera on 6/21/24.
//

import SwiftUI

struct OrangeGradientBackgroundView<Content: View>: View {
    @ViewBuilder
    let content: Content

    var body: some View {
        ZStack {
            RadialGradient(
                gradient: Gradient(colors: [
                    Color.red.opacity(0.9),
                    Color.orange.opacity(0.6),
                ]),
                center: .center, startRadius: 2, endRadius: 650
            )
            .edgesIgnoringSafeArea(.all)

            content
        }
    }
}

#Preview {
    OrangeGradientBackgroundView {
        Text("Hello").foregroundStyle(.white)
    }
}
