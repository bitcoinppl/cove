//
//  VerifyWordsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

extension Bip39AutoComplete: AutoComplete {}

struct VerifyWordsView: View {
    var model: WalletViewModel
    var groupedWords: [[GroupedWord]]
    @State private var tabIndex = 0

    var body: some View {
        TabView(selection: $tabIndex) {
            ForEach(self.groupedWords, id: \.self) { wordGroup in
                CardTab(wordGroup: wordGroup)
            }
        }
        .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
    }
}

struct CardTab: View {
    let wordGroup: [GroupedWord]
    @State private var fields = ["", "", "", "", "", ""]

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
            Text("\(self.word.number). ")

            TextField("Enter Word", text: self.$text)
                .focused(self.$isFocused)
                .onChange(of: self.text) {
                    if self.text.lowercased() == self.word.word.lowercased() {
                        self.showSuggestions = false
                        return
                    }

                    self.showSuggestions = !self.text.isEmpty && self.isFocused && !self.filteredSuggestions.isEmpty
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
    }

    var filteredSuggestions: [String] {
        autocompleter.autocomplete(word: text)
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
        VerifyWordsView(model: model, groupedWords: model.rust.bip39WordsGrouped())
}
