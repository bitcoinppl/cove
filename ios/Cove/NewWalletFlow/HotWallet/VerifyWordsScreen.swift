//
//  VerifyWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

struct VerifyWordsScreen: View {
    let id: WalletId

    @Environment(\.navigate) private var navigate
    @Environment(MainViewModel.self) private var appModel
    @State var model: WalletViewModel? = nil

    // private
    @State private var tabIndex: Int = 0

    @State private var invalidWords: String = ""
    @State private var focusField: Int?

    @State private var validator: WordValidator? = nil
    @State private var groupedWords: [[GroupedWord]] = [[]]
    @State private var enteredWords: [[String]] = [[]]
    @State private var textFields: [String] = []
    @State private var filteredSuggestions: [String] = []

    @StateObject private var keyboardObserver = KeyboardObserver()

    // alerts
    private enum AlertType: Identifiable {
        case error, words, skip
        var id: Self { self }
    }

    @State private var activeAlert: AlertType?

    func initOnAppear() {
        do {
            let model = try WalletViewModel(id: id)
            let validator = try model.rust.wordValidator()
            let groupedWords = validator.groupedWords()

            self.model = model
            self.validator = validator
            self.groupedWords = groupedWords
            enteredWords = groupedWords.map { $0.map { _ in "" }}
        } catch {
            Log.error("VerifyWords failed to initialize: \(error)")
        }
    }

    var keyboardIsShowing: Bool {
        keyboardObserver.keyboardIsShowing
    }

    var cardHeight: CGFloat {
        keyboardIsShowing ? 325 : 425
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

    private func DisplayAlert(for alertType: AlertType) -> Alert {
        switch alertType {
        case .error:
            Alert(
                title: Text("Words not valid"),
                message: Text("The following words are not valid: \(invalidWords)"),
                dismissButton: .cancel(Text("OK"))
            )
        case .words:
            Alert(
                title: Text("See Secret Words?"),
                message: Text("Whoever has your secret words has access to your bitcoin. Please keep these safe and don't show them to anyone else."),
                primaryButton: .destructive(Text("Yes, Show Me")) {
                    appModel.pushRoute(Route.secretWords(id))
                },
                secondaryButton: .cancel(Text("Cancel"))
            )
        case .skip:
            Alert(
                title: Text("Skip verifying words?"),
                message: Text("Are you sure you want to skip verifying words? Without having a back of these words, you could lose your bitcoin"),
                primaryButton: .destructive(Text("Yes, Verify Later")) {
                    Log.debug("Skipping verification, going to wallet id: \(id)")
                    appModel.resetRoute(to: Route.selectedWallet(id))
                },
                secondaryButton: .cancel(Text("Cancel"))
            )
        }
    }

    func confirm(_ model: WalletViewModel, _ validator: WordValidator) {
        guard isAllWordsValid else {
            activeAlert = .error
            invalidWords = validator.invalidWordsString(enteredWords: enteredWords)
            return
        }

        do {
            try model.rust.markWalletAsVerified()
            appModel.resetRoute(to: Route.selectedWallet(id))
        } catch {
            Log.error("Error marking wallet as verified: \(error)")
        }
    }

    var body: some View {
        if let model = model, let validator = validator {
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
                        VStack {
                            TabView(selection: $tabIndex) {
                                ForEach(Array(validator.groupedWords().enumerated()), id: \.offset) { index, wordGroup in
                                    VStack {
                                        CardTab(wordGroup: wordGroup, fields: $enteredWords[index], filteredSuggestions: $filteredSuggestions, focusField: $focusField)
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
                    .toolbar {
                        ToolbarItemGroup(placement: .keyboard) {
                            HStack {
                                ForEach(filteredSuggestions, id: \.self) { word in
                                    Spacer()
                                    Button(word) {
                                        guard let focusField = focusField else { return }
                                        let (outerIndex, remainder) = focusField.quotientAndRemainder(dividingBy: 6)
                                        let innerIndex = remainder - 1
                                        enteredWords[outerIndex][innerIndex] = word
                                        self.focusField = focusField + 1
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
                    .padding(.horizontal, 30)

                    Spacer()

                    if tabIndex == lastIndex {
                        Button("Confirm") {
                            confirm(model, validator)
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
                        activeAlert = .words
                    }) {
                        Text("View Words")
                            .font(.subheadline)
                            .fontWeight(.semibold)
                            .foregroundStyle(.opacity(0.8))
                    }
                    .padding(.top, 10)

                    Button(action: {
                        activeAlert = .skip
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
            .alert(item: $activeAlert) { alertType in
                DisplayAlert(for: alertType)
            }
            .onChange(of: focusField) { _, _ in
                filteredSuggestions = []
            }
        } else {
            Text("Loading....")
                .onAppear(perform: initOnAppear)
        }
    }
}

private struct CardTab: View {
    let wordGroup: [GroupedWord]
    @Binding var fields: [String]
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
            ForEach(Array(self.wordGroup.enumerated()), id: \.offset) { index, word in
                AutocompleteField(autocomplete: Bip39AutoComplete(),
                                  word: word,
                                  text: self.$fields[index],
                                  filteredSuggestions: $filteredSuggestions,
                                  focusField: self.$focusField)
            }
        }
    }
}

private struct AutocompleteField: View {
    let autocomplete: Bip39AutoComplete
    let word: GroupedWord

    @Binding var text: String
    @Binding var filteredSuggestions: [String]
    @Binding var focusField: Int?

    @State private var showSuggestions = false
    @State private var offset: CGPoint = .zero
    @FocusState private var isFocused: Bool

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

    func submitFocusField() {
        filteredSuggestions = []
        guard let focusField = focusField else {
            return
        }

        self.focusField = focusField + 1
    }

    var textField: some View {
        TextField("", text: $text,
                  prompt: Text("enter secret word...")
                      .foregroundColor(.white.opacity(0.65)))
            .foregroundColor(borderColor ?? .white)
            .frame(alignment: .trailing)
            .padding(.trailing, 8)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled(true)
            .keyboardType(.asciiCapable)
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
                filteredSuggestions = autocomplete.autocomplete(word: text)

                if self.filteredSuggestions.count == 1 && self.filteredSuggestions.first == word.word {
                    self.text = self.filteredSuggestions.first!

                    submitFocusField()
                    return
                }
            }
    }
}

#Preview {
    VerifyWordsScreen(id: WalletId())
        .environment(MainViewModel())
}
