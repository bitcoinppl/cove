//
//  VerifyWordsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

// WEDNESDAY TODO:
// 1. Add ability to click on disabled confirm button
// 2. Add confirm button on last page, if not correct, say which ones are not good!

struct VerifyWordsView: View {
    var model: WalletViewModel
    var groupedWords: [[GroupedWord]]

    @State private var enteredWords: [[String]]
    @State private var tabIndex: Int

    @StateObject private var keyboardObserver = KeyboardObserver()

    init() {
        // TODO: get wallet id, and wallet model from id
        model = WalletViewModel(numberOfWords: .twelve)
        groupedWords = model.rust.bip39WordsGrouped()

        enteredWords = groupedWords.map { _ in Array(repeating: "", count: 6) }
        tabIndex = 0
    }

    var keyboardIsShowing: Bool {
        keyboardObserver.keyboardIsShowing
    }

    var cardHeight: CGFloat {
        keyboardIsShowing ? 350 : 450
    }

    var buttonIsDisabled: Bool {
        !model.rust.isValidWordGroup(groupNumber: UInt8(tabIndex), enteredWords: enteredWords[tabIndex])
    }

    var isAllWordsValid: Bool {
        model.rust.isAllWordsValid(enteredWords: enteredWords)
    }

    var lastIndex: Int {
        groupedWords.count - 1
    }

    var body: some View {
        SunsetWave {
            VStack {
                Spacer()

                if !keyboardIsShowing {
                    Text("Please verify your words")
                        .font(.title2)
                        .fontWeight(.medium)
                        .foregroundColor(.white.opacity(0.85))
                        .padding(.top, 60)
                        .padding(.bottom, 30)
                }

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
                .frame(height: cardHeight)
                .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
                .padding(.horizontal, 30)

                Spacer()

                if tabIndex == lastIndex {
                    Button("Confirm") {
                        // TODO: confirm
                    }
                    .buttonStyle(GradientButtonStyle(disabled: !isAllWordsValid))
                    .padding(.top, 20)

                } else {
                    Button("Next") {
                        withAnimation {
                            tabIndex += 1
                        }
                    }
                    .buttonStyle(GlassyButtonStyle(disabled: buttonIsDisabled))
                    .disabled(buttonIsDisabled)
                    .foregroundStyle(Color.red)
                    .padding(.top, 20)
                }

                Spacer()
            }
        }
        .environment(model)
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

struct CardTab: View {
    let wordGroup: [GroupedWord]
    @Binding var fields: [String]
    @State var focusField: Int?

    func zIndex(index: Int) -> Double {
        // if focused, on the bottom half, don't set zIndex
        // becuase we want the suggestions to show on top
        if let field = focusField, field > 3 {
            return 1
        }

        return 6 - Double(index)
    }

    var body: some View {
        VStack(spacing: 20) {
            ForEach(Array(self.wordGroup.enumerated()), id: \.offset) { index, word in
                AutocompleteField(autocompleter: Bip39AutoComplete(),
                                  text: self.$fields[index],
                                  word: word,
                                  focusField: self.$focusField)
                    .zIndex(zIndex(index: index))
            }

        }.onAppear {
            print(self.wordGroup)
        }
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

struct AutocompleteField<AutoCompleter: AutoComplete>: View {
    let autocompleter: AutoCompleter
    @Binding var text: String

    let word: GroupedWord

    @Binding var focusField: Int?

    @Environment(WalletViewModel.self) private var model
    @State private var showSuggestions = false
    @State private var offset: CGPoint = .zero
    @FocusState private var isFocused: Bool

    var filteredSuggestions: [String] {
        autocompleter.autocomplete(word: text)
    }

    var frameHeight: CGFloat {
        CGFloat(filteredSuggestions.count) * 50
    }

    var borderColor: Color? {
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

    var body: some View {
        HStack {
            Text("\(String(format: "%02d", self.word.number)). ")
                .foregroundColor(.secondary)

            textField
                .overlay(alignment: Alignment(horizontal: .center, vertical: .top)) {
                    Group {
                        if self.showSuggestions {
                            SuggestionList(suggestions: self.filteredSuggestions, selection: self.$text)
                                .transition(.move(edge: .top))
                                .frame(height: frameHeight)
                                .offset(y: word.number <= 3 ? 0 : -60 - frameHeight)
                        }
                    }
                    .offset(y: 40)
                    .zIndex(20)
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
        .enableInjection()
    }

    var textField: some View {
        TextField("", text: $text,
                  prompt: Text("Placeholder text")
                      .foregroundColor(.white.opacity(0.65)))
            .foregroundColor(borderColor ?? .white)
            .frame(alignment: .trailing)
            .padding(.trailing, 8)
            .textInputAutocapitalization(.never)
            .focused($isFocused)
            .onChange(of: isFocused) {
                if !self.isFocused { return self.showSuggestions = false }

                if isFocused {
                    focusField = Int(word.number)
                }
            }
            .onSubmit {
                model.submitWordField(fieldNumber: word.number)
            }
            .onChange(of: model.focusField) { _, fieldNumber in
                guard let fieldNumber = fieldNumber else { return }
                if word.number == fieldNumber {
                    isFocused = true
                }
            }
            .onChange(of: text) {
                if !self.isFocused {
                    return self.showSuggestions = false
                }

                if self.text.lowercased() == self.word.word {
                    return self.showSuggestions = false
                }

                if self.filteredSuggestions.count == 1 {
                    self.showSuggestions = false
                }

                if self.filteredSuggestions.count == 1 && self.filteredSuggestions.first == word.word {
                    self.showSuggestions = false
                    self.text = self.filteredSuggestions.first!
                    model.submitWordField(fieldNumber: word.number)
                    return
                }

                self.showSuggestions = !self.text.isEmpty && !self.filteredSuggestions.isEmpty
            }
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
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
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    VerifyWordsView()
}
