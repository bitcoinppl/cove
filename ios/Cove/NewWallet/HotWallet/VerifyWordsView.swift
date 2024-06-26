//
//  VerifyWordsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

struct VerifyWordsView: View {
    let walletId: WalletId
    let model: WalletViewModel
    let validator: WordValidator?
    let groupedWords: [[GroupedWord]]

    @Environment(\.navigate) private var navigate
    @State private var enteredWords: [[String]]
    @State private var tabIndex: Int

    @State private var showErrorAlert = false
    @State private var invalidWords: String = ""
    @State private var focusField: Int?
    @State private var showSkipAlert = false

    @StateObject private var keyboardObserver = KeyboardObserver()

    init(id: WalletId) {
        walletId = id
        model = WalletViewModel(id: id)

        var validator: WordValidator?

        do {
            validator = try model.rust.wordValidator()
        } catch {
            // TODO: handle error better?, show error alert?
            print("[SWIFT] Unable to create word validator: \(error)")
        }

        groupedWords = validator?.groupedWords() ?? []
        enteredWords = groupedWords.map { _ in Array(repeating: "", count: 6) }
        tabIndex = 0
        focusField = nil
        showSkipAlert = false

        self.validator = validator
    }

    var keyboardIsShowing: Bool {
        keyboardObserver.keyboardIsShowing
    }

    var cardHeight: CGFloat {
        keyboardIsShowing ? 350 : 450
    }

    var buttonIsDisabled: Bool {
        !validator!.isValidWordGroup(groupNumber: UInt8(tabIndex), enteredWords: enteredWords[tabIndex])
    }

    var isAllWordsValid: Bool {
        validator!.isAllWordsValid(enteredWords: enteredWords)
    }

    var lastIndex: Int {
        groupedWords.count - 1
    }

    var body: some View {
        if let validator = validator {
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

                    FixedGlassCard {
                        TabView(selection: $tabIndex) {
                            ForEach(Array(self.groupedWords.enumerated()), id: \.offset) { index, wordGroup in
                                CardTab(wordGroup: wordGroup, fields: $enteredWords[index], focusField: $focusField)
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
                            if isAllWordsValid {
                                // confirm
                            } else {
                                showErrorAlert = true
                                invalidWords = validator.invalidWordsString(enteredWords: enteredWords)
                            }
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

                    Button(action: {
                        showSkipAlert = true
                    }) {
                        Text("SKIP")
                            .font(.subheadline)
                            .fontWeight(.semibold)
                            .foregroundStyle(.opacity(0.6))
                    }
                    .padding(.top, 10)

                    Spacer()
                }
            }
            .alert("Words not valid", isPresented: $showErrorAlert) {
                Button("OK", role: .cancel) {}
            } message: {
                Text("The following words are not valid: \(invalidWords)")
            }
            .alert(isPresented: $showSkipAlert) {
                Alert(
                    title: Text("Skip verifying words?"),
                    message: Text("Are you sure you want to skip verifying words? Without having a back of these words, you could lose your bitcoin"),
                    primaryButton: .destructive(Text("Yes, Verify Later")) {
                        navigate(Route.listWallets)
                    },
                    secondaryButton: .cancel(Text("Cancel"))
                )
            }
            .enableInjection()
        } else {
            // TODO: handle better
            Text("No words found")
        }
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

struct CardTab: View {
    let wordGroup: [GroupedWord]
    @Binding var fields: [String]
    @Binding var focusField: Int?

    func zIndex(index: Int) -> Double {
        // if focused, on the bottom half, don't set zIndex
        // because we want the suggestions to show on top
        if let field = focusField, (field % 6) == 0 || (field % 6) > 3 {
            return 1
        }

        return 6 - Double(index)
    }

    var body: some View {
        VStack(spacing: 20) {
            ForEach(Array(self.wordGroup.enumerated()), id: \.offset) { index, word in
                AutocompleteField(autocompleter: Bip39AutoComplete(),
                                  word: word,
                                  text: self.$fields[index],
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
    let word: GroupedWord

    @Binding var text: String
    @Binding var focusField: Int?

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

    var offsetCalc: CGFloat {
        // bottom half word, show suggestions above the word
        if word.number % 6 == 0 || word.number % 6 > 3 {
            return -60 - frameHeight
        }

        // top half word, show suggestions below the word
        return 0
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
                                .offset(y: offsetCalc)
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

    func submitFocusField() {
        guard let focusField = focusField else {
            return
        }

        self.focusField = focusField + 1
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
                submitFocusField()
            }
            .onChange(of: focusField) { _, fieldNumber in
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
                    submitFocusField()
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
                .foregroundColor(.black.opacity(0.75))
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
    VerifyWordsView(id: WalletId())
}
