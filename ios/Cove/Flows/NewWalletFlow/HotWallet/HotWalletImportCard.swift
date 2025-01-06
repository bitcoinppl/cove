//
//  HotWalletImportCard.swift
//  Cove
//
//  Created by Praveen Perera on 1/6/25.
//

import SwiftUI

private let rowHeight = 30.0
private let numberOfRows = 6

struct HotWalletImportCard: View {
    var numberOfWords: NumberOfBip39Words

    @Binding var tabIndex: Int
    @Binding var enteredWords: [[String]]
    @Binding var filteredSuggestions: [String]
    @Binding var focusField: Int?

    @ViewBuilder
    var MainContent: some View {
        VStack(spacing: 0) {
            TabView(selection: $tabIndex) {
                ForEach(Array(enteredWords.enumerated()), id: \.offset) { index, _ in
                    CardTab(
                        fields: $enteredWords[index],
                        groupIndex: index,
                        filteredSuggestions: $filteredSuggestions,
                        focusField: $focusField,
                        allEnteredWords: enteredWords,
                        numberOfWords: numberOfWords
                    )
                    .tag(index)
                }
                .frame(maxWidth: .infinity)
            }
        }
        .frame(maxWidth: .infinity)
    }

    var body: some View {
        GroupBox {
            MainContent
        }
        .tabViewStyle(PageTabViewStyle(indexDisplayMode: .never))
        .cornerRadius(10)
        .frame(maxHeight: rowHeight * CGFloat(numberOfRows) + 100)
    }
}

private struct CardTab: View {
    @Binding var fields: [String]
    let groupIndex: Int
    @Binding var filteredSuggestions: [String]
    @Binding var focusField: Int?

    let allEnteredWords: [[String]]
    let numberOfWords: NumberOfBip39Words

    let cardSpacing: CGFloat = 20

    var rows: [GridItem] {
        Array(repeating: .init(.fixed(rowHeight)), count: numberOfRows)
    }

    var body: some View {
        GeometryReader { proxy in
            LazyHGrid(rows: rows, spacing: cardSpacing) {
                ForEach(Array(fields.enumerated()), id: \.offset) { index, _ in
                    AutocompleteField(
                        number: (groupIndex * 6) + (index + 1),
                        autocomplete: Bip39WordSpecificAutocomplete(
                            wordNumber: UInt16((groupIndex * 6) + (index + 1)),
                            numberOfWords: numberOfWords
                        ),
                        allEnteredWords: allEnteredWords,
                        numberOfWords: numberOfWords,
                        text: $fields[index],
                        filteredSuggestions: $filteredSuggestions,
                        focusField: $focusField
                    )
                }
                .frame(width: (proxy.size.width / 2) - (cardSpacing / 2))
            }
            .frame(maxWidth: .infinity)
        }
    }
}

private struct AutocompleteField: View {
    let number: Int
    let autocomplete: Bip39WordSpecificAutocomplete
    let allEnteredWords: [[String]]
    let numberOfWords: NumberOfBip39Words

    @Binding var text: String
    @Binding var filteredSuggestions: [String]
    @Binding var focusField: Int?

    @State private var state: FieldState = .initial
    @State private var showSuggestions = false
    @State private var offset: CGPoint = .zero
    @FocusState private var isFocused: Bool

    private enum FieldState {
        case initial
        case valid
        case invalid
    }

    var borderColor: Color? {
        switch state {
        case .initial: .none
        case .valid: Color.green.opacity(0.6)
        case .invalid: Color.red.opacity(0.7)
        }
    }

    var textColor: Color {
        switch state {
        case .initial:
            .secondary.opacity(0.45)
        case .valid:
            .green.opacity(0.8)
        case .invalid:
            .red
        }
    }

    var body: some View {
        HStack {
            Text("\(String(format: "%d", number)).".padLeft(with: " ", toLength: 3))
                .font(.subheadline)
                .fontDesign(.monospaced)
                .foregroundColor(textColor)
                .lineLimit(1)
                .minimumScaleFactor(0.75)
                .fixedSize(horizontal: true, vertical: true)
                .frame(alignment: .leading)

            ZStack(alignment: .centerFirstTextBaseline) {
                Line()
                    .stroke(textColor, lineWidth: 1)
                    .frame(height: 1)
                    .frame(maxWidth: .infinity)
                    .padding(.trailing, 5)

                textField
                    .offset(y: -2)
            }
        }
        .onAppear {
            if !text.isEmpty, autocomplete.isBip39Word(word: text) {
                state = .valid
            }
        }
        .frame(maxWidth: .infinity)
    }

    func submitFocusField() {
        filteredSuggestions = []
        guard let focusField else { return }

        if autocomplete.isValidWord(word: text, allWords: allEnteredWords) {
            state = .valid
        } else {
            state = .invalid
        }

        self.focusField = min(focusField + 1, numberOfWords.toWordCount())
    }

    var textField: some View {
        TextField("", text: $text)
            .font(.subheadline)
            .foregroundColor(textColor)
            .frame(alignment: .trailing)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled(true)
            .keyboardType(.asciiCapable)
            .focused($isFocused)
            .onChange(of: isFocused) {
                if !isFocused {
                    showSuggestions = false
                    return
                }

                filteredSuggestions = autocomplete.autocomplete(
                    word: text, allWords: allEnteredWords
                )

                if isFocused { focusField = number }
            }
            .onSubmit {
                submitFocusField()
            }
            .onChange(of: focusField) { _, fieldNumber in
                guard let fieldNumber else { return }
                if number == fieldNumber {
                    isFocused = true
                }
            }
            .onChange(of: text) { oldText, newText in
                filteredSuggestions = autocomplete.autocomplete(
                    word: newText, allWords: allEnteredWords
                )

                if oldText.count > newText.count {
                    // erasing, reset state
                    state = .initial
                }

                // empty is always initial
                if newText == "" {
                    state = .initial
                    return
                }

                // invalid, no words match
                if filteredSuggestions.isEmpty {
                    state = .invalid
                    return
                }

                // if only one suggestion left and if we added a letter (not backspace)
                // then auto select the first selection, because we want auto selection
                // but also allow the user to fix a wrong word
                if let word = filteredSuggestions.last,
                   filteredSuggestions.count == 1, oldText.count < newText.count
                {
                    state = .valid
                    filteredSuggestions = []

                    if text != word {
                        text = word
                        submitFocusField()
                        return
                    }
                }
            }
            .onAppear {
                if let focusField, focusField == number {
                    isFocused = true
                }
            }
    }
}

#Preview {
    struct Container: View {
        @State var tabIndex: Int = 0
        @State var enteredWords: [[String]] = Array(repeating: Array(repeating: "", count: 12), count: 2)
        @State var filteredSuggestions: [String] = []
        @State var focusField: Int? = nil

        var body: some View {
            HotWalletImportCard(
                numberOfWords: .twelve,
                tabIndex: $tabIndex,
                enteredWords: $enteredWords,
                filteredSuggestions: $filteredSuggestions,
                focusField: $focusField
            )
        }
    }

    return Container()
}
