//
//  SendFlowDurationCapsule.swift
//  Cove
//
//  Created by Praveen Perera on 1/20/25.
//
import SwiftUI

struct SendFlowDurationCapsule: View {
    let speed: FeeSpeed
    let fontColor: Color
    var font: Font = .subheadline
    var fontWeight: Font.Weight = .regular

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(speed.circleColor)
                .frame(width: 8, height: 8)
            Text(speed.duration)
        }
        .font(font)
        .fontWeight(fontWeight)
        .foregroundColor(fontColor)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Color.gray.opacity(0.2))
        .cornerRadius(8)
    }
}

#Preview {
    VStack {
        SendFlowDurationCapsule(speed: .slow, fontColor: Color.primary)
    }
}
