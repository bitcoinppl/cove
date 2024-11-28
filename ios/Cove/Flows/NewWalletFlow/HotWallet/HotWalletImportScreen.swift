//
//  HotWalletImportScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

struct HotWalletImportScreen: View {
    // public
    let autocomplete = Bip39AutoComplete()
    @State var numberOfWords: NumberOfBip39Words
    @State var importType: ImportType = .manual

    // private
    @Environment(\.navigate) private var navigate
    @Environment(MainViewModel.self) private var app

    @State private var isPresentingScanner = false
    @State private var tabIndex: Int = 0

    @State private var showErrorAlert = false
    @State private var duplicateWallet: DuplicateWalletItem? = .none

    @State private var focusField: Int?

    @StateObject private var keyboardObserver = KeyboardObserver()

    @State var model: ImportWalletViewModel = .init()
    @State private var validator: WordValidator? = nil

    @State var enteredWords: [[String]] = [[]]
    @State var filteredSuggestions: [String] = []

    // qr code scanning
    @Environment(\.dismiss) var dismiss
    @State private var multiQr: MultiQr?
    @State private var scannedCode: TaggedString?
    @State private var scanComplete: Bool = false
    @State private var scanError: TaggedString?

    // nfc scanning
    @State private var nfcReader: NFCReader = .init()
    @State private var tasks: [Task<Void, any Error>] = []

    func initOnAppear() {
        nfcReader = NFCReader()

        switch importType {
        case .manual: ()
        case .qr: isPresentingScanner = true
        case .nfc:
            let task = Task {
                try await Task.sleep(for: .milliseconds(200))
                await MainActor.run {
                    nfcReader.scan()
                }
            }

            tasks.append(task)
        }

        importType = .manual
        enteredWords = numberOfWords.inGroups()
    }

    var keyboardIsShowing: Bool {
        keyboardObserver.keyboardIsShowing
    }

    var cardHeight: CGFloat {
        keyboardIsShowing ? 350 : 450
    }

    var buttonIsDisabled: Bool {
        enteredWords[tabIndex].map { word in autocomplete.isValidWord(word: word) }.contains(
            false)
    }

    var isAllWordsValid: Bool {
        !enteredWords.joined().map { word in autocomplete.isValidWord(word: word) }.contains(
            false)
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

    private func handleScan(result: Result<ScanResult, ScanError>) {
        if case let .failure(error) = result {
            Log.error("Scan error: \(error.localizedDescription)")
            dismiss()
            return
        }

        guard case let .success(scanResult) = result else { return }
        let qr = StringOrData(scanResult.data)

        do {
            let multiQr: MultiQr =
                try multiQr
                    ?? {
                        let newMultiQr = try MultiQr.tryNew(qr: qr)
                        self.multiQr = newMultiQr
                        return newMultiQr
                    }()

            // see if its single qr or seed qr
            if let words = try multiQr.getGroupedWords(qr: qr, groupsOf: UInt8(6)) {
                setWords(words)
            }
        } catch {
            Log.error("Seed QR failed to scan: \(error.localizedDescription)")
            scanError = TaggedString(error.localizedDescription)
            isPresentingScanner = false

            // reset multiqr on error
            multiQr = nil
        }
    }

    func importWallet() {
        do {
            let walletMetadata = try model.rust.importWallet(enteredWords: enteredWords)
            try app.rust.selectWallet(id: walletMetadata.id)
            app.resetRoute(to: .selectedWallet(walletMetadata.id))
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                self.showErrorAlert = true
            case let .WalletAlreadyExists(walletId):
                duplicateWallet = DuplicateWalletItem(id: UUID(), walletId: walletId)
            case let .WalletImportError(error):
                Log.error("Import error: \(error)")
            case let .KeychainError(keychainError):
                Log.error("Unable to save wallet to keychain: \(keychainError)")
            case let .DatabaseError(databaseError):
                Log.error("Unable to save wallet metadata to database: \(databaseError)")
            case let .BdkError(error):
                Log.error("Unable to import wallet: \(error)")
            }
        } catch {
            Log.error("Unknown error \(error)")
        }
    }

    @ViewBuilder
    var KeyboardAutoCompleteView: some View {
        HStack {
            ForEach(filteredSuggestions, id: \.self) { word in
                Spacer()
                Button(word) {
                    guard let focusFieldUnchecked = focusField else { return }

                    let focusField = min(focusFieldUnchecked, numberOfWords.toWordCount())
                    var (outerIndex, remainder) = focusField.quotientAndRemainder(dividingBy: 6)
                    var innerIndex = remainder - 1

                    // adjust for last word
                    if innerIndex < 0 {
                        innerIndex = 5
                        outerIndex = outerIndex - 1
                    }

                    if innerIndex > 5 || outerIndex > lastIndex || outerIndex < 0 || innerIndex < 0 {
                        Log.error(
                            "Something went wrong: innerIndex: \(innerIndex), outerIndex: \(outerIndex), lastIndex: \(lastIndex), focusField: \(focusField)"
                        )
                        return
                    }

                    enteredWords[outerIndex][innerIndex] = word

                    // if its not the last word, go to next focusField
                    self.focusField = min(focusField + 1, numberOfWords.toWordCount())
                    filteredSuggestions = []
                }
                .foregroundColor(.secondary)
                Spacer()

                // only show divider in the middle
                if filteredSuggestions.count > 1, filteredSuggestions.last != word {
                    Divider()
                }
            }
        }
    }

    @ViewBuilder
    var NextOrImportButton: some View {
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
    }

    @ViewBuilder
    var MainContent: some View {
        VStack {
            TabView(selection: $tabIndex) {
                ForEach(Array(enteredWords.enumerated()), id: \.offset) { index, _ in
                    VStack {
                        CardTab(
                            fields: $enteredWords[index],
                            groupIndex: index,
                            filteredSuggestions: $filteredSuggestions,
                            focusField: $focusField,
                            allEnteredWords: enteredWords,
                            numberOfWords: numberOfWords
                        )
                        .tag(index)
                        .padding(.bottom, keyboardIsShowing ? 60 : 20)
                    }
                }
                .padding(.horizontal, 30)
            }
        }
    }

    var body: some View {
        VStack {
            Spacer()

            GroupBox {
                MainContent
            }
            .frame(height: cardHeight)
            .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
            .navigationTitle("Import Wallet")
            .toolbarColorScheme(.dark, for: .navigationBar)
            .navigationBarTitleDisplayMode(navDisplay)
            .toolbar {
                ToolbarItemGroup(placement: .keyboard) {
                    KeyboardAutoCompleteView
                }
            }
            .padding(.top, keyboardIsShowing ? 80 : 0)
            .padding(.horizontal, 30)
            .cornerRadius(20)

            Spacer()

            NextOrImportButton.padding(.bottom, 24)

            HStack {
                Button(action: {
                    isPresentingScanner = true
                }) {
                    HStack {
                        Image(systemName: "qrcode.viewfinder")
                            .font(.system(size: 20))

                        Text("QR")
                    }
                    .padding()
                    .frame(minWidth: 60)
                    .foregroundColor(.white)
                    .background(
                        LinearGradient(
                            gradient: Gradient(colors: [
                                .black.opacity(0.7), .black, .black.opacity(0.8),
                            ]),
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                    )
                    .cornerRadius(10)
                    .shadow(color: .gray.opacity(0.5), radius: 5, x: 0, y: 2)
                }
                .buttonStyle(PlainButtonStyle())

                Button(action: {
                    nfcReader.scan()
                }) {
                    HStack {
                        Image(systemName: "wave.3.right")
                            .font(.system(size: 18))

                        Text("NFC")
                    }
                    .padding()
                    .frame(minWidth: 60)
                    .foregroundColor(.white)
                    .background(
                        LinearGradient(
                            gradient: Gradient(colors: [
                                .black.opacity(0.7), .black, .black.opacity(0.8),
                            ]),
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                    )
                    .cornerRadius(10)
                    .shadow(color: .gray.opacity(0.5), radius: 5, x: 0, y: 2)
                }
                .buttonStyle(PlainButtonStyle())
            }

            Spacer()
        }
        .onChange(of: focusField) {
            filteredSuggestions = []
        }
        .alert("Words not valid", isPresented: $showErrorAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(
                "The words you entered does not create a valid wallet. Please check the words and try again."
            )
        }
        .alert(item: $duplicateWallet) { duplicate in
            Alert(
                title: Text("Duplicate Wallet"),
                message: Text("This wallet has already been imported!"),
                dismissButton: .default(Text("OK")) {
                    try? app.rust.selectWallet(id: duplicate.walletId)
                    app.resetRoute(to: .selectedWallet(duplicate.walletId))
                }
            )
        }
        .alert(item: $scanError) { error in
            Alert(
                title: Text("Error Scanning QR Code"),
                message: Text(error.value),
                dismissButton: .default(Text("OK")) {
                    scanError = nil
                }
            )
        }
        .sheet(isPresented: $isPresentingScanner) {
            ScannerView(
                codeTypes: [.qr],
                scanMode: .oncePerCode,
                scanInterval: 0.1
            ) { response in
                handleScan(result: response)
            }
        }
        .onAppear(perform: initOnAppear)
        .onChange(of: enteredWords) {
            // if its the last word on the non last card and all words are valid words, then go to next tab
            // focusField will already have changed by now
            if let focusField,
               !buttonIsDisabled, tabIndex < lastIndex, focusField % 6 == 1
            {
                withAnimation {
                    tabIndex += 1
                }
            }
        }
        .onChange(of: nfcReader.scannedMessage) { _, msg in
            guard let msg else { return }
            do {
                let words = try groupedPlainWordsOf(mnemonic: msg, groups: 6)
                setWords(words)
            } catch {
                Log.error("Error NFC word parsing: \(error)")
            }
        }

        .onChange(of: nfcReader.scannedMessageData) { _, data in
            // received data, probably a SeedQR in NFC
            guard let data else { return }
            do {
                let seedQR = try SeedQr.newFromData(data: data)
                let words = seedQR.groupedPlainWords()
                setWords(words)
            } catch {
                Log.error("Error NFC word parsing from data: \(error)")
            }
        }
        .onDisappear {
            nfcReader.resetReader()
            nfcReader.session = nil

            for task in tasks {
                task.cancel()
            }
        }
    }

    func setWords(_ words: [[String]]) {
        let numberOfWords = words.compactMap(\.count).reduce(0, +)
        switch numberOfWords {
        case 12: self.numberOfWords = .twelve
        case 24: self.numberOfWords = .twentyFour
        default:
            Log.warn("Invalid number of words: \(numberOfWords)")
            scanError = TaggedString(
                "Invalid number of words: \(numberOfWords), we only support 12 or 24 words")
            isPresentingScanner = false
            return
        }

        // reset multiqr and nfc reader on succesful scan
        multiQr = nil
        nfcReader.resetReader()
        nfcReader.session = nil

        enteredWords = words
        isPresentingScanner = false
        tabIndex = lastIndex
    }
}

private struct CardTab: View {
    @Binding var fields: [String]
    let groupIndex: Int
    @Binding var filteredSuggestions: [String]
    @Binding var focusField: Int?

    let allEnteredWords: [[String]]
    let numberOfWords: NumberOfBip39Words

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
            }
        )
        .onAppear {
            if !text.isEmpty, autocomplete.isBip39Word(word: text) {
                state = .valid
            }
        }
    }

    func submitFocusField() {
        filteredSuggestions = []
        guard let focusField else {
            return
        }

        if autocomplete.isValidWord(word: text, allWords: allEnteredWords) {
            state = .valid
        } else {
            state = .invalid
        }

        self.focusField = min(focusField + 1, numberOfWords.toWordCount())
    }

    var textField: some View {
        TextField(
            "", text: $text,
            prompt: Text("enter secret word...")
                .foregroundColor(.secondary)
        )
        .foregroundColor(textColor)
        .frame(alignment: .trailing)
        .padding(.trailing, 8)
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled(true)
        .keyboardType(.asciiCapable)
        .focused($isFocused)
        .onChange(of: isFocused) {
            if !isFocused {
                showSuggestions = false
                return
            }

            filteredSuggestions = autocomplete.autocomplete(word: text, allWords: allEnteredWords)

            if isFocused {
                focusField = number
            }
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

private struct DuplicateWalletItem: Identifiable {
    var id: UUID
    var walletId: WalletId
}

#Preview("12 Words") {
    NavigationStack {
        HotWalletImportScreen(numberOfWords: .twelve)
            .environment(MainViewModel())
    }
}

#Preview("24 Words") {
    NavigationStack {
        HotWalletImportScreen(numberOfWords: .twentyFour)
            .environment(MainViewModel())
    }
}
