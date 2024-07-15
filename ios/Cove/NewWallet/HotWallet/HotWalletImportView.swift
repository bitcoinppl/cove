//
//  HotWalletImportView.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

struct HotWalletImportView: View {
    let autocomplete = Bip39AutoComplete()
    @State var numberOfWords: NumberOfBip39Words

    @Environment(\.navigate) private var navigate
    @Environment(MainViewModel.self) private var appModel

    @State private var tabIndex: Int = 0

    @State private var showErrorAlert = false
    @State private var focusField: Int?

    @StateObject private var keyboardObserver = KeyboardObserver()

    @State var model: ImportWalletViewModel = .init()
    @State private var validator: WordValidator? = nil

    @State var enteredWords: [[String]] = [[]]
    @State var filteredSuggestions: [String] = []

    func initOnAppear() {
        enteredWords = numberOfWords.inGroups()
        print("enteredWords: \(enteredWords)")
    }

    var keyboardIsShowing: Bool {
        keyboardObserver.keyboardIsShowing
    }

    var cardHeight: CGFloat {
        keyboardIsShowing ? 350 : 450
    }

    var buttonIsDisabled: Bool {
        return enteredWords[tabIndex].map { word in autocomplete.isValidWord(word: word) }.contains(false)
    }

    var isAllWordsValid: Bool {
        return !enteredWords.joined().map { word in autocomplete.isValidWord(word: word) }.contains(false)
    }

    var navDisplay: NavigationBarItem.TitleDisplayMode {
        withAnimation {
            keyboardIsShowing ? .inline : .large
        }
    }

    var lastIndex: Int {
        switch numberOfWords {
        case .twelve:
            1
        case .twentyFour:
            3
        }
    }

    func importWallet() {
        print("import wallet")
    }

    var body: some View {
        VStack {
            Spacer()

            GroupBox {
                VStack {
                    TabView(selection: $tabIndex) {
                        ForEach(Array(enteredWords.enumerated()), id: \.offset) { index, _ in
                            VStack {
                                CardTab(fields: $enteredWords[index], groupIndex: index, filteredSuggestions: $filteredSuggestions, focusField: $focusField)
                                    .tag(index)
                                    .padding(.bottom, keyboardIsShowing ? 60 : 20)
                            }
                        }
                        .padding(.horizontal, 30)
                    }
                }
            }
            .frame(height: cardHeight)
            .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
            .navigationTitle("Import Wallet")
            .navigationBarTitleDisplayMode(navDisplay)
            .toolbar {
                ToolbarItemGroup(placement: .keyboard) {
                    HStack {
                        ForEach(filteredSuggestions, id: \.self) { word in
                            Spacer()
                            Button(word) {
                                guard let focusField = focusField else { return }

                                var (outerIndex, remainder) = focusField.quotientAndRemainder(dividingBy: 6)
                                var innerIndex = remainder - 1

                                // adjust for last word
                                if innerIndex < 0 {
                                    innerIndex = 5
                                    outerIndex = outerIndex - 1
                                }

                                if innerIndex > 5 || outerIndex > lastIndex || outerIndex < 0 || innerIndex < 0 {
                                    return
                                }

                                enteredWords[outerIndex][innerIndex] = word
                                self.focusField = focusField + 1
                                filteredSuggestions = []
                            }
                            .foregroundColor(.secondary)
                            Spacer()

                            // only show divider in the middle
                            if filteredSuggestions.count > 1 && filteredSuggestions.last != word {
                                Divider()
                            }
                        }
                    }
                }
            }
            .padding(.top, keyboardIsShowing ? 80 : 0)
            .padding(.horizontal, 30)

            Spacer()

            if tabIndex == lastIndex {
                Button("Import") { importWallet() }
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
        .alert("Words not valid", isPresented: $showErrorAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("The words you entered does not create a valid wallet. Please check the words and try again.")
        }
        .onChange(of: focusField) { _, _ in
            filteredSuggestions = []
        }
        .enableInjection()
        .onAppear(perform: initOnAppear)
        .onChange(of: enteredWords) {
            if !buttonIsDisabled && tabIndex < lastIndex {
                withAnimation {
                    tabIndex += 1
                }
            }
        }
    }
}

private struct CardTab: View {
    @Binding var fields: [String]
    let groupIndex: Int
    @Binding var filteredSuggestions: [String]
    @Binding var focusField: Int?

    @StateObject private var keyboardObserver = KeyboardObserver()

    var keyboardIsShowing: Bool {
        keyboardObserver.keyboardIsShowing
    }

    var cardSpacing: CGFloat {
        keyboardIsShowing ? 15 : 20
    }

    var body: some View {
        VStack(spacing: cardSpacing) {
            ForEach(Array(fields.enumerated()), id: \.offset) { index, _ in
                AutocompleteField(
                    number: (groupIndex * 6) + (index + 1),
                    autocomplete: Bip39AutoComplete(),
                    text: $fields[index],
                    filteredSuggestions: $filteredSuggestions,
                    focusField: self.$focusField)
            }
        }
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

private struct AutocompleteField: View {
    let number: Int
    let autocomplete: Bip39AutoComplete

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
            .secondary
        case .valid:
            .green.opacity(0.8)
        case .invalid:
            .red
        }
    }

    var body: some View {
        HStack {
            Text("\(String(format: "%02d", number)). ")
                .foregroundColor(.secondary)

            textField
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
        filteredSuggestions = []
        guard let focusField = focusField else {
            return
        }

        if autocomplete.isValidWord(word: text) {
            state = .valid
        } else {
            state = .invalid
        }

        self.focusField = focusField + 1
    }

    var textField: some View {
        TextField("", text: $text,
                  prompt: Text("enter secret word...")
                      .foregroundColor(.secondary))
            .foregroundColor(textColor)
            .frame(alignment: .trailing)
            .padding(.trailing, 8)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled(true)
            .keyboardType(.asciiCapable)
            .focused($isFocused)
            .onChange(of: isFocused) {
                if !self.isFocused { return self.showSuggestions = false }

                if isFocused {
                    focusField = number
                }
            }
            .onSubmit {
                submitFocusField()
            }
            .onChange(of: focusField) { _, fieldNumber in
                guard let fieldNumber = fieldNumber else { return }
                if number == fieldNumber {
                    isFocused = true
                }
            }
            .onChange(of: text) { oldText, newText in
                filteredSuggestions = autocomplete.autocomplete(word: newText)

                if oldText.count > newText.count {
                    // erasing, reset state
                    state = .initial
                }

                // empty is always initial
                if newText == "" {
                    return state = .initial
                }

                // invalid, no words match
                if filteredSuggestions.isEmpty {
                    return state = .invalid
                }

                // if only one suggestion left and if we added a letter (not backspace)
                // then auto select the first selection, because we want auto selection
                // but also allow the user to fix a wrong word
                if let word = filteredSuggestions.last, filteredSuggestions.count == 1 && oldText.count < newText.count {
                    if self.text != word {
                        self.text = word
                        submitFocusField()
                        return
                    }
                }
            }
            .onAppear {
                if let focusField = self.focusField, focusField == number {
                    isFocused = true
                }
            }
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

#Preview {
    VerifyWordsView(id: WalletId())
}
