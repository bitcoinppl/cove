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

    var body: some View {
        Capsule()
            .fill(color.opacity(0.2))
            .frame(width: 130, height: 30)
            .overlay(
                HStack {
                    Image(systemName: icon)
                        .font(.system(size: 12))
                        .foregroundColor(color)
                    Text(text)
                        .font(.system(size: 14))
                        .foregroundColor(color)
                }
            )
    }
}

#Preview {
    TransactionCapsule(text: "Received", icon: "arrow.up.right", color: .green)
}
