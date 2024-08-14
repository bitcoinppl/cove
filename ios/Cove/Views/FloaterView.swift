//
//  f.swift
//  Cove
//
//  Created by Praveen Perera on 8/14/24.
//

import SwiftUI

struct FloaterView: View {
    // required
    let text: String

    // optional
    let backgroundColor = Color.black
    let textColor = Color.white
    let iconColor = Color.green
    let icon = "checkmark"

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: icon)
                .foregroundColor(iconColor)
                .frame(width: 24, height: 24)

            Text(text)
                .foregroundColor(textColor)
        }
        .padding(16)
        .padding(.horizontal, 24)
        .background(.black)
        .cornerRadius(12)
    }
}

#Preview {
    FloaterView(text: "Address Copied")
}
