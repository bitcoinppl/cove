//
//  TransactionCapsuleView.swift
//  Cove
//
//  Created by Praveen Perera on 9/15/24.
//

import SwiftUI

struct TransactionCapsule: View {
    var text: String
    var icon: String

    var color: Color
    var textColor: Color?

    var _textColor: Color {
        textColor ?? color
    }

    var capsuleColor: Color {
        if textColor != nil {
            color
        } else {
            color.opacity(0.2)
        }
    }

    var body: some View {
        Capsule()
            .fill(capsuleColor)
            .frame(width: 130, height: 30)
            .overlay(
                HStack {
                    Image(systemName: icon)
                        .font(.system(size: 12))
                        .foregroundColor(_textColor)
                    Text(text)
                        .font(.system(size: 14))
                        .foregroundColor(_textColor)
                }
            )
    }
}

#Preview {
    TransactionCapsule(text: "Received", icon: "arrow.up.right", color: .green)
}
