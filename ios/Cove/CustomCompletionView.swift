//
//  CustomCompletionView.swift
//  Cove
//
//  Created by Praveen Perera on 7/11/24.
//

import SwiftUI

struct CustomCompletionView: View {
    @Binding var text: String
    @State private var completions: [String] = []

    var body: some View {
        VStack {
            TextField("Enter text", text: $text)
                .autocorrectionDisabled(true)
                .keyboardType(.asciiCapable)
                .onChange(of: text) { _, newValue in
                    updateCompletions(for: newValue)
                }

            ScrollView(.horizontal, showsIndicators: false) {
                HStack {
                    ForEach(completions, id: \.self) { completion in
                        Button(completion) {
                            text = completion
                        }
                        .padding(.horizontal)
                    }
                }
            }
        }
    }

    func updateCompletions(for input: String) {
        // Implement your custom completion logic here
        // This is where you'd generate completions based on the current input
        completions = ["Example", "Completions", "Based on", input]
    }
}

#Preview {
    CustomCompletionView(text: Binding.constant("test"))
}
