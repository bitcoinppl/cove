//
//  HotWalletImportScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

private let rowHeight = 30.0
private let numberOfRows = 6

private enum AlertState: Equatable {
    case invalidWords
    case duplicateWallet(WalletId)
    case scanError(String)
}

private enum SheetState: Equatable {
    case qrCode
}

struct HotWalletImportScreen: View {
    // public
    let autocomplete = Bip39AutoComplete()
    @State var numberOfWords: NumberOfBip39Words
    @State var importType: ImportType = .manual

    // private
    @Environment(\.navigate) private var navigate
    @Environment(AppManager.self) private var app

    @State private var tabIndex: Int = 0
    @State private var duplicateWallet: DuplicateWalletItem? = .none

    @State private var focusField: Int?

    @State var manager: ImportWalletManager = .init()
    @State private var validator: WordValidator? = nil

    @State var enteredWords: [[String]] = [[]]
    @State var filteredSuggestions: [String] = []

    // alerts & sheets
    @State private var alertState: TaggedItem<AlertState>? = .none
    @State private var sheetState: TaggedItem<SheetState>? = .none

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
        case .qr: sheetState = .init(.qrCode)
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
        enteredWords = numberOfWords.inGroups(of: 12)
    }

    var buttonIsDisabled: Bool {
        enteredWords[tabIndex].map { word in autocomplete.isValidWord(word: word) }.contains(
            false)
    }

    var isAllWordsValid: Bool {
        !enteredWords.joined().map { word in autocomplete.isValidWord(word: word) }.contains(
            false)
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
            sheetState = .none

            // reset multiqr on error
            multiQr = nil
        }
    }

    func importWallet() {
        do {
            let walletMetadata = try manager.rust.importWallet(enteredWords: enteredWords)
            try app.rust.selectWallet(id: walletMetadata.id)
            app.resetRoute(to: .selectedWallet(walletMetadata.id))
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                alertState = .init(.invalidWords)
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
    var ImportButton: some View {
        Button("Import wallet") {
            importWallet()
        }
        .font(.subheadline)
        .fontWeight(.medium)
        .frame(maxWidth: .infinity)
        .contentShape(Rectangle())
        .padding(.vertical, 20)
        .background(Color.btnPrimary)
        .foregroundColor(.midnightBlue)
        .cornerRadius(10)
    }

    @ViewBuilder
    var MainContent: some View {
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
        }
    }

    @ToolbarContentBuilder
    var ToolbarContent: some ToolbarContent {
        ToolbarItemGroup(placement: .keyboard) {
            KeyboardAutoCompleteView
        }

        ToolbarItem(placement: .principal) {
            Text("Import Wallet")
                .font(.callout)
                .fontWeight(.semibold)
                .foregroundStyle(.white)
        }

        ToolbarItemGroup(placement: .topBarTrailing) {
            HStack(spacing: 5) {
                Button(action: nfcReader.scan) {
                    Image(systemName: "wave.3.right")
                        .font(.subheadline)
                        .foregroundColor(.white)
                }

                Button(action: { sheetState = .init(.qrCode) }) {
                    Image(systemName: "qrcode.viewfinder")
                        .font(.subheadline)
                        .foregroundColor(.white)
                }
            }
        }
    }

    var body: some View {
        VStack {
            Spacer()

            GroupBox {
                MainContent
            }
            .tabViewStyle(PageTabViewStyle(indexDisplayMode: .never))
            .cornerRadius(10)
            .frame(maxHeight: rowHeight * CGFloat(numberOfRows) + 100)

            if numberOfWords == .twentyFour {
                DotMenuView(selected: tabIndex, size: 5, total: 2)
            }

            Spacer()

            ImportButton
        }
        .padding()
        .padding(.bottom, 24)
        .toolbar { ToolbarContent }
        .sheet(item: $sheetState, content: SheetContent)
        .alert(alertTitle, isPresented: showingAlert, presenting: alertState, actions: { MyAlert($0).actions })
        .onAppear(perform: initOnAppear)
        .onChange(of: focusField) { filteredSuggestions = [] }
        .onChange(of: enteredWords, onChangeEnteredWords)
        .onChange(of: nfcReader.scannedMessage, onChangeScannedMessage)
        .onChange(of: nfcReader.scannedMessageData, onChangeScannedMessageData)
        .onDisappear {
            nfcReader.resetReader()
            nfcReader.session = nil
            for task in tasks {
                task.cancel()
            }
        }
        .background(
            Image(.newWalletPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: screenHeight * 0.75, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .opacity(0.5)
        )
        .background(Color.midnightBlue)
        .tint(.white)
    }

    // MARK: Alerts

    private var showingAlert: Binding<Bool> {
        Binding(
            get: { alertState != nil },
            set: { if !$0 { alertState = .none } }
        )
    }

    private var alertTitle: String {
        guard let alertState else { return "Error" }
        return MyAlert(alertState).title
    }

    private func MyAlert(_ alert: TaggedItem<AlertState>) -> some AlertBuilderProtocol {
        let singleOkCancel = {
            Button("Ok", role: .cancel) {
                alertState = .none
            }
        }

        switch alert.item {
        case .invalidWords:
            return AlertBuilder(
                title: "Words not valid",
                message: "The words you entered does not create a valid wallet. Please check the words and try again.",
                actions: singleOkCancel
            )
        case let .duplicateWallet(walletId):
            return AlertBuilder(
                title: "Duplicate Wallet",
                message: "This wallet has already been imported!",
                actions: {
                    Button("OK", role: .cancel) {
                        alertState = .none
                        try? app.rust.selectWallet(id: walletId)
                        app.resetRoute(to: .selectedWallet(walletId))
                    }
                }
            )
        case let .scanError(error):
            return AlertBuilder(
                title: "Error Scanning QR Code",
                message: error,
                actions: singleOkCancel
            )
        }
    }

    // MARK: Sheet

    @ViewBuilder
    private func SheetContent(_ state: TaggedItem<SheetState>) -> some View {
        switch state.item {
        case .qrCode:
            ScannerView(
                codeTypes: [.qr],
                scanMode: .oncePerCode,
                scanInterval: 0.1
            ) { response in
                handleScan(result: response)
            }
        }
    }

    // MARK: OnChange Functions

    func onChangeEnteredWords(_: [[String]]?, _: [[String]]?) {
        // if its the last word on the non last card and all words are valid words, then go to next tab
        // focusField will already have changed by now
        if let focusField,
           !buttonIsDisabled, tabIndex < lastIndex, focusField % 12 == 1
        {
            withAnimation {
                tabIndex += 1
            }
        }
    }

    func onChangeScannedMessage(_: String?, _ msg: String?) {
        guard let msg else { return }
        do {
            let words = try groupedPlainWordsOf(mnemonic: msg, groups: 6)
            setWords(words)
        } catch {
            Log.error("Error NFC word parsing: \(error)")
        }
    }

    func onChangeScannedMessageData(_: Data?, _ data: Data?) {
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

    func setWords(_ words: [[String]]) {
        let numberOfWords = words.compactMap(\.count).reduce(0, +)
        switch numberOfWords {
        case 12: self.numberOfWords = .twelve
        case 24: self.numberOfWords = .twentyFour
        default:
            Log.warn("Invalid number of words: \(numberOfWords)")
            scanError = TaggedString(
                "Invalid number of words: \(numberOfWords), we only support 12 or 24 words")

            sheetState = .none
            return
        }

        // reset multiqr and nfc reader on succesful scan
        multiQr = nil
        nfcReader.resetReader()
        nfcReader.session = nil

        enteredWords = words
        sheetState = .none
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

    let cardSpacing: CGFloat = 20

    var rows: [GridItem] {
        Array(repeating: .init(.fixed(rowHeight)), count: numberOfRows)
    }

    var body: some View {
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
            Text("\(String(format: "%d", number)). ")
                .font(.subheadline)
                .foregroundColor(.secondary)

            Spacer()

            textField
        }
        .onAppear {
            if !text.isEmpty, autocomplete.isBip39Word(word: text) {
                state = .valid
            }
        }
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

private struct DuplicateWalletItem: Identifiable {
    var id: UUID
    var walletId: WalletId
}

#Preview("12 Words") {
    NavigationStack {
        HotWalletImportScreen(numberOfWords: .twelve)
            .environment(AppManager())
    }
}

#Preview("24 Words") {
    NavigationStack {
        HotWalletImportScreen(numberOfWords: .twentyFour)
            .environment(AppManager())
    }
}
