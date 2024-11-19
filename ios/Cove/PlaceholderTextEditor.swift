//
//  PlaceholderTextEditor.swift
//  Cove
//
//  Created by Praveen Perera on 11/7/24.
//

import SwiftUI

struct PlaceholderTextEditor: View {
    @Binding var text: String
    let placeholder: String

    var body: some View {
        ZStack(alignment: .topLeading) {
            TextEditor(text: $text)

            if text.isEmpty {
                Text(placeholder)
                    .foregroundStyle(.tertiary)
                    .padding(.top, 8)
                    .padding(.leading, 5)
            }
        }
    }
}
