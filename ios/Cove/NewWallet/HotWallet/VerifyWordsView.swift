//
//  VerifyWordsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

// MONDAY TODO:
// 1. Add disabled next button
// 2. Add confirm button on last page, if not correct, say which ones are not good!

struct VerifyWordsView: View {
    var model: WalletViewModel
    var groupedWords: [[GroupedWord]]

    @State private var enteredWords: [[String]]
    @State private var tabIndex: Int

    init(model: WalletViewModel, groupedWords: [[GroupedWord]]) {
        self.model = model
        self.groupedWords = groupedWords
        self.enteredWords = groupedWords.map { _ in Array(repeating: "", count: 6) }
        self.tabIndex = 0
    }

    var body: some View {
        GlassCard {
            TabView(selection: $tabIndex) {
                ForEach(Array(self.groupedWords.enumerated()), id: \.offset) { index, wordGroup in
                    CardTab(wordGroup: wordGroup, fields: $enteredWords[index])
                        .tag(index)
                }
                .padding(.horizontal, 30)
                .padding(.vertical, 30)
            }
        }
        .frame(height: 500)
        .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
    }
}

struct CardTab: View {
    let wordGroup: [GroupedWord]
    @Binding var fields: [String]

    var body: some View {
        VStack(spacing: 20) {
            ForEach(Array(self.wordGroup.enumerated()), id: \.offset) { index, word in
                AutocompleteField(autocompleter: Bip39AutoComplete(), text: self.$fields[index], word: word)
                    .zIndex(6 - Double(index))
            }
        }.onAppear {
            print(self.wordGroup)
        }
    }
}

struct AutocompleteField<AutoCompleter: AutoComplete>: View {
    let autocompleter: AutoCompleter
    @Binding var text: String

    let word: GroupedWord

    @State private var showSuggestions = false
    @State private var offset: CGPoint = .zero
    @FocusState private var isFocused: Bool

    var body: some View {
        HStack {
            Text("\(String(format: "%02d", self.word.number)). ")
                .foregroundColor(.secondary)

            TextField("", text: $text,
                      prompt: Text("Placeholder text")
                          .foregroundColor(.white.opacity(0.65)))
                .foregroundColor(borderColor ?? .white)
                .frame(alignment: .trailing)
                .padding(.trailing, 8)
                .textInputAutocapitalization(.never)
                .focused(self.$isFocused)
                .onChange(of: self.isFocused) {
                    if !self.isFocused {
                        return self.showSuggestions = false
                    }
                }
                .onChange(of: self.text) {
                    if !self.isFocused {
                        return self.showSuggestions = false
                    }

                    if self.text.lowercased() == self.word.word {
                        return self.showSuggestions = false
                    }

                    if self.filteredSuggestions.count == 1 && self.filteredSuggestions.first == word.word {
                        self.showSuggestions = false
                        return self.text = self.filteredSuggestions.first!
                    }

                    self.showSuggestions = !self.text.isEmpty && !self.filteredSuggestions.isEmpty
                }
                .overlay(alignment: Alignment(horizontal: .center, vertical: .top)) {
                    Group {
                        if self.showSuggestions {
                            SuggestionList(suggestions: self.filteredSuggestions, selection: self.$text)
                                .transition(.move(edge: .top))
                                .frame(height: CGFloat(self.filteredSuggestions.count) * 50)
                                .zIndex(10)
                        }
                    }
                    .offset(y: 40)
                }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .overlay(
            Group {
                if let color = borderColor {
                    RoundedRectangle(cornerRadius: 10)
                        .stroke(color, lineWidth: 2)
                }
            })
    }

    var filteredSuggestions: [String] {
        autocompleter.autocomplete(word: text)
    }

    var borderColor: Color? {
        print("text is \(text), \(isFocused), \(filteredSuggestions.count), \(word.word)")

        // starting state
        if text == "" {
            return .none
        }

        // correct
        if text.lowercased() == word.word {
            return Color.green.opacity(0.8)
        }

        // focused and not the only suggestion
        if isFocused && filteredSuggestions.count > 1 {
            return .none
        }

        // focused, but no other possibilities left
        if isFocused && filteredSuggestions.isEmpty {
            return Color.red.opacity(0.8)
        }

        // wrong word, not focused
        if text.lowercased() != word.word {
            return Color.red.opacity(0.8)
        }

        return .none
    }
}

struct SuggestionList: View {
    let suggestions: [String]
    @Binding var selection: String

    var body: some View {
        List(suggestions, id: \.self) { suggestion in
            Text(suggestion)
                .onTapGesture {
                    self.selection = suggestion
                }
                .padding(.vertical, 4)
        }
        .listStyle(.inset)
        .cornerRadius(10)
        .shadow(radius: 5)
        .padding(.trailing, 20)
    }
}

#Preview {
    @State var model = WalletViewModel(numberOfWords: .twelve)

    return
        SunsetWave {
            VStack {
                Spacer()
                VerifyWordsView(model: model, groupedWords: model.rust.bip39WordsGrouped())
                    .padding()
                    .padding(.horizontal, 20)
                Spacer()
            }
        }
}
