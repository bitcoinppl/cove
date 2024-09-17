//
//  ConfirmationIndicatorView.swift
//  Cove
//
//  Created by Praveen Perera on 9/16/24.
//

import SwiftUI

struct ConfirmationIndicatorView: View {
    let current: Int
    let total: Int = 3

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Confirmations")
                .foregroundColor(.secondary)

            Text("\(current) of \(total)")
                .fontWeight(.bold)

            HStack(spacing: 10) {
                ForEach(0 ..< total, id: \.self) { index in
                    RoundedRectangle(cornerRadius: 4)
                        .fill(index < current ? Color.green : Color.secondary.opacity(0.3))
                        .frame(height: 8)
                }
            }
        }
        .padding()
    }
}

#Preview("0") {
    ConfirmationIndicatorView(current: 0)
}

#Preview("1") {
    ConfirmationIndicatorView(current: 1)
}

#Preview("2") {
    ConfirmationIndicatorView(current: 2)
}
