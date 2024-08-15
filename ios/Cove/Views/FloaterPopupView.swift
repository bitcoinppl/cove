//
//  f.swift
//  Cove
//
//  Created by Praveen Perera on 8/14/24.
//

import SwiftUI

struct FloaterPopupView: View {
    // required
    let text: String

    // optional
    var backgroundColor: Color = .black
    var textColor: Color = .white
    var iconColor: Color = .green
    var icon: String = "checkmark"

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
    FloaterPopupView(text: "Address Copied")
}
