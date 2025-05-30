//
//  HotWalletImportCard.swift
//  Cove
//
//  Created by Praveen Perera on 1/6/25.
//

import SwiftUI

private let rowHeight = 30.0
private let numberOfRows = 6

private let groupsOf = HotWalletImportScreen.GROUPS_OF

struct HotWalletImportCard: View {
    var numberOfWords: NumberOfBip39Words

    @Binding var tabIndex: Int
    @Binding var enteredWords: [[String]]
    @Binding var filteredSuggestions: [String]

    @FocusState.Binding var focusField: ImportFieldNumber?

    @ViewBuilder
    var MainContent: some View {
        VStack(spacing: 0) {
            TabView(selection: $tabIndex) {
                ForEach(Array(enteredWords.enumerated()), id: \.offset) { index, _ in
                    CardTab(
                        fields: $enteredWords[index],
                        tabIndex: $tabIndex,
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
    @Binding var tabIndex: Int
    let groupIndex: Int
    @Binding var filteredSuggestions: [String]

    @FocusState.Binding var focusField: ImportFieldNumber?

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
                        number: (groupIndex * groupsOf) + (index + 1),
                        autocomplete: Bip39WordSpecificAutocomplete(
                            wordNumber: UInt16((groupIndex * groupsOf) + (index + 1)),
                            numberOfWords: numberOfWords
                        ),
                        allEnteredWords: allEnteredWords,
                        numberOfWords: numberOfWords,
                        tabIndex: $tabIndex,
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
    @Environment(\.colorScheme) var colorScheme

    let number: Int
    let autocomplete: Bip39WordSpecificAutocomplete
    let allEnteredWords: [[String]]
    let numberOfWords: NumberOfBip39Words

    @Binding var tabIndex: Int
    @Binding var text: String
    @Binding var filteredSuggestions: [String]
    @FocusState.Binding var focusField: ImportFieldNumber?

    @State private var state: FieldState = .initial
    @State private var showSuggestions = false
    @State private var offset: CGPoint = .zero

    private enum FieldState {
        case initial
        case typing
        case valid
        case invalid
    }

    var borderColor: Color? {
        switch state {
        case .initial: .none
        case .typing: .none
        case .valid: Color.green.opacity(0.6)
        case .invalid: Color.red.opacity(0.7)
        }
    }

    var textColor: Color {
        switch state {
        case .initial:
            .secondary.opacity(0.45)
        case .typing:
            .primary
        case .valid:
            .green.opacity(0.8)
        case .invalid:
            .red
        }
    }

    var numberColor: Color {
        switch state {
        case .initial:
            .secondary.opacity(0.45)
        default:
            .secondary
        }
    }

    var isFocused: Bool {
        if focusField == nil { return number == 1 }
        return focusField == ImportFieldNumber(number)
    }

    var body: some View {
        HStack {
            Text("\(String(format: "%d", number)).".padLeft(with: " ", toLength: 3))
                .font(.subheadline)
                .fontDesign(.monospaced)
                .foregroundColor(numberColor)
                .lineLimit(1)
                .minimumScaleFactor(0.75)
                .fixedSize(horizontal: true, vertical: true)
                .frame(alignment: .leading)

            ZStack(alignment: .centerFirstTextBaseline) {
                if state == .initial || state == .typing {
                    Line()
                        .stroke(textColor, lineWidth: 1)
                        .frame(height: 1)
                        .frame(maxWidth: .infinity)
                        .padding(.trailing, 5)
                }

                textField
                    .offset(y: state == .typing ? -4 : 0)
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

        let currentFocusField = UInt8(focusField?.fieldNumber ?? 1)
        let nextFieldNumber = Int(
            autocomplete.nextFieldNumber(
                currentFieldNumber: currentFocusField,
                enteredWords: allEnteredWords.flatMap(\.self)
            ))

        focusField = ImportFieldNumber(nextFieldNumber)

        if (nextFieldNumber % groupsOf) == 1 {
            withAnimation {
                tabIndex = Int(nextFieldNumber / groupsOf)
            }
        }

        if text == "" { return state = .typing }
        if autocomplete.isValidWord(word: text, allWords: allEnteredWords) { return state = .valid }
        state = .invalid
    }

    var textField: some View {
        TextField("", text: $text)
            .font(.subheadline)
            .fontWeight(.bold)
            .foregroundColor(textColor)
            .frame(alignment: .trailing)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled(true)
            .keyboardType(.asciiCapable)
            .focused($focusField, equals: ImportFieldNumber(number))
            .tint(colorScheme == .dark ? .white : .black)
            .onChange(of: isFocused) {
                if !isFocused {
                    showSuggestions = false
                    return
                }

                filteredSuggestions = autocomplete.autocomplete(
                    word: text, allWords: allEnteredWords
                )
            }
            .onChange(of: focusField) { _, _ in
                if text == "" { return }

                if autocomplete.isValidWord(word: text, allWords: allEnteredWords) {
                    state = .valid
                } else {
                    state = .invalid
                }
            }
            .onSubmit { submitFocusField() }
            .onChange(of: text, initial: false) { oldText, newText in
                text = newText.trimmingCharacters(in: .whitespacesAndNewlines)

                filteredSuggestions = autocomplete.autocomplete(
                    word: newText, allWords: allEnteredWords
                )

                // initial set to typing
                if newText.count > oldText.count { state = .typing }

                // erasing, reset state to typing
                if oldText.count > newText.count, !filteredSuggestions.contains(newText) { return state = .typing }

                // set to valid if it matches a word
                if filteredSuggestions.contains(newText) { state = .valid }

                // empty is always initial, or typing
                if newText.isEmpty, isFocused { return state = .typing }
                if newText.isEmpty, !isFocused { return state = .initial }

                // invalid, no words match
                if filteredSuggestions.isEmpty { return state = .invalid }

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
    }
}

#Preview {
    struct Container: View {
        @State var tabIndex: Int = 0
        @State var enteredWords: [[String]] = Array(repeating: Array(repeating: "", count: 12), count: 2)
        @State var filteredSuggestions: [String] = []
        @FocusState var focusField: ImportFieldNumber?

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
