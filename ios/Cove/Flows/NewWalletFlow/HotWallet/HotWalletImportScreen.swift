//
//  HotWalletImportScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

private let groupsOf = HotWalletImportScreen.GROUPS_OF

private enum AlertState: Equatable {
    case invalidWords
    case duplicateWallet(WalletId)
    case scanError(String)
}

private enum SheetState: Equatable {
    case qrCode
}

enum ImportFieldNumber: Int, Hashable, CaseIterable {
    case one
    case two
    case three
    case four
    case five
    case six
    case seven
    case eight
    case nine
    case ten
    case eleven
    case twelve
    case thirteen
    case fourteen
    case fifteen
    case sixteen
    case seventeen
    case eighteen
    case nineteen
    case twenty
    case twentyOne
    case twentyTwo
    case twentyThree
    case twentyFour

    ///  0 index, covertr to field number
    var fieldNumber: Int {
        rawValue + 1
    }

    init(_ fieldNumber: Int) {
        self = Self(rawValue: fieldNumber - 1) ?? .one
    }

    init(_ fieldNumber: UInt8) {
        self = .init(Int(fieldNumber))
    }
}

/// consts
extension HotWalletImportScreen {
    static let GROUPS_OF = 12
}

struct HotWalletImportScreen: View {
    @Environment(\.sizeCategory) var sizeCategory

    // public
    let autocomplete = Bip39AutoComplete()
    @State var numberOfWords: NumberOfBip39Words
    @State var importType: ImportType = .manual

    // private
    @Environment(\.navigate) private var navigate
    @Environment(AppManager.self) private var app

    @State private var tabIndex: Int = 0
    @FocusState var focusField: ImportFieldNumber?

    @State private var keyboardObserver = KeyboardObserver()

    private var accessoryHeight: CGFloat {
        switch sizeCategory {
        case .extraSmall, .small, .medium: 46
        case .large: 48
        case .extraLarge: 50
        case .extraExtraLarge: 52
        default: 54
        }
    }

    private var accessoryMinWidth: CGFloat {
        let maxWidth = screenWidth - 32
        let target: CGFloat = 240 // three-chip feel
        return min(maxWidth, target)
    }

    @State var manager: ImportWalletManager = .init()
    @State private var validator: WordValidator? = nil

    @State var enteredWords: [[String]] = [[]]
    @State var filteredSuggestions: [String] = []

    // alerts & sheets
    @State private var alertState: TaggedItem<AlertState>? = .none
    @State private var sheetState: TaggedItem<SheetState>? = .none

    // qr code scanning
    @Environment(\.dismiss) var dismiss
    @State private var scanner = QrScanner()
    @State private var scannedCode: TaggedString?
    @State private var scanComplete: Bool = false
    @State private var scanError: TaggedString?

    // nfc scanning
    @State private var nfcReader: NFCReader = .init()
    @State private var tasks: [Task<Void, any Error>] = []

    func initOnAppear() {
        nfcReader = NFCReader()

        switch importType {
        case .manual:
            let task = Task {
                try await Task.sleep(for: .milliseconds(800))
                focusField = .one
            }
            tasks.append(task)
        case .qr:
            sheetState = .init(.qrCode)
        case .nfc:
            let task = Task {
                try await Task.sleep(for: .milliseconds(200))
                await MainActor.run {
                    nfcReader.scan()
                }
            }

            tasks.append(task)
        }

        enteredWords = numberOfWords.inGroups(of: groupsOf)
    }

    var buttonIsDisabled: Bool {
        enteredWords[tabIndex].map { word in autocomplete.isValidWord(word: word) }.contains(
            false
        )
    }

    var isAllWordsValid: Bool {
        !enteredWords.joined().map { word in autocomplete.isValidWord(word: word) }.contains(
            false
        )
    }

    var lastIndex: Int {
        switch numberOfWords {
        case .twelve:
            0
        case .twentyFour:
            1
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
            switch try scanner.scan(qr: qr) {
            case let .complete(multiFormat, haptic):
                haptic.trigger()
                // extract mnemonic words from the result
                if case let .mnemonic(mnemonic) = multiFormat {
                    let mnemonicString = mnemonic.words().joined(separator: " ")
                    if let words = try? groupedPlainWordsOf(mnemonic: mnemonicString, groups: UInt8(groupsOf)) {
                        setWords(words)
                    }
                } else {
                    sheetState = .none
                    app.alertState = TaggedItem(.general(
                        title: "Invalid QR Code",
                        message: "Please scan a valid seed phrase QR code"
                    ))
                }
                scanner.reset()

            case let .inProgress(_, haptic):
                // multi-part QR in progress - keep scanning
                haptic.trigger()
            }
        } catch {
            Log.error("Seed QR failed to scan: \(error.localizedDescription)")
            sheetState = .none
            scanner.reset()
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
                alertState = .init(.duplicateWallet(walletId))
            case let .WalletImportError(error):
                Log.error("Import error: \(error)")
            case let .Keychain(keychainError):
                Log.error("Unable to save wallet to keychain: \(keychainError)")
            case let .Database(databaseError):
                Log.error("Unable to save wallet metadata to database: \(databaseError)")
            case let .BdkError(error):
                Log.error("Unable to import wallet: \(error)")
            case let .MissingMetadata(walletId):
                Log.error("Wallet metadata missing for \(walletId)")
            }
        } catch {
            Log.error("Unknown error \(error)")
        }
    }

    func selectWordInKeyboard(_ word: String) {
        let focusFieldNumber = min(focusField?.fieldNumber ?? 1, numberOfWords.toWordCount())

        var (outerIndex, remainder) = focusFieldNumber.quotientAndRemainder(dividingBy: groupsOf)
        var innerIndex = remainder - 1

        // adjust for last word
        if innerIndex < 0 {
            innerIndex = groupsOf - 1
            outerIndex = outerIndex - 1
        }

        if innerIndex >= groupsOf || outerIndex > lastIndex || outerIndex < 0 || innerIndex < 0 {
            Log.error(
                "Something went wrong: innerIndex: \(innerIndex), outerIndex: \(outerIndex), lastIndex: \(lastIndex), focusField: \(focusFieldNumber)"
            )
            return
        }

        enteredWords[outerIndex][innerIndex] = word
        filteredSuggestions = []

        let newFocusFieldNumber = Int(
            autocomplete.nextFieldNumber(
                currentFieldNumber: UInt8(focusFieldNumber),
                enteredWords: enteredWords.flatMap(\.self)
            )
        )

        focusField = ImportFieldNumber(newFocusFieldNumber)

        // going to new page
        if (newFocusFieldNumber % groupsOf) == 1 {
            tabIndex = Int(newFocusFieldNumber / groupsOf)
        }
    }

    var KeyboardAutoCompleteView: some View {
        HStack {
            ForEach(filteredSuggestions, id: \.self) { word in
                Spacer()
                Button(word, action: { selectWordInKeyboard(word) })
                    .foregroundColor(.primary)

                Spacer()

                // only show divider in the middle
                if filteredSuggestions.count > 1, filteredSuggestions.last != word {
                    Divider()
                }
            }
        }
        .frame(maxWidth: .infinity)
    }

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

    @ToolbarContentBuilder
    var ToolbarContent: some ToolbarContent {
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
        GeometryReader { geometry in
            let bottomSafeArea = geometry.safeAreaInsets.bottom

            ZStack(alignment: .bottom) {
                MainContent

                if keyboardObserver.keyboardIsShowing, focusField != nil, !filteredSuggestions.isEmpty {
                    KeyboardToolbar
                        .offset(y: -(keyboardObserver.keyboardHeight - bottomSafeArea))
                        .transition(.opacity)
                        .animation(.easeInOut(duration: 0.2), value: filteredSuggestions.isEmpty)
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .ignoresSafeArea(.keyboard)
    }

    private var KeyboardToolbar: some View {
        HStack {
            ForEach(filteredSuggestions, id: \.self) { word in
                Spacer()
                Button(word) { selectWordInKeyboard(word) }
                    .foregroundColor(.primary)
                Spacer()

                if filteredSuggestions.count > 1, filteredSuggestions.last != word {
                    Divider()
                }
            }
        }
        .frame(maxWidth: .infinity)
        .frame(height: accessoryHeight)
        .background(.regularMaterial)
        .modifier(KeyboardToolbarShapeModifier())
    }

    var Card: some View {
        HotWalletImportCard(
            numberOfWords: numberOfWords,
            onPasteMnemonic: handlePasteMnemonic,
            tabIndex: $tabIndex,
            enteredWords: $enteredWords,
            filteredSuggestions: $filteredSuggestions,
            focusField: $focusField
        )
    }

    var MainContent: some View {
        VStack {
            if !keyboardObserver.keyboardIsShowing {
                Spacer()
            }

            if isMiniDeviceOrLargeText(sizeCategory) {
                ScrollView {
                    Card
                        .frame(idealHeight: 300)
                }
                .scrollIndicators(.hidden)
            } else {
                Card
            }

            if numberOfWords == .twentyFour {
                DotMenuView(selected: tabIndex, size: 5, total: 2)
            }

            Spacer()

            ImportButton
        }
        .animation(.easeInOut(duration: 0.25), value: keyboardObserver.keyboardIsShowing)
        .padding()
        .padding(.bottom, 24)
        .toolbar { ToolbarContent }
        .sheet(item: $sheetState, content: SheetContent)
        .alert(
            alertTitle,
            isPresented: showingAlert,
            presenting: alertState,
            actions: { MyAlert($0).actions },
            message: { MyAlert($0).message }
        )
        .modifier(ConditionalTintModifier())
        .onAppear(perform: initOnAppear)
        .onChange(of: sheetState, initial: true) { oldState, newState in
            if oldState != nil, newState == nil {
                if enteredWords[0][0] == "" {
                    focusField = ImportFieldNumber(0)
                    return
                }

                let focusField =
                    autocomplete.nextFieldNumber(
                        currentFieldNumber: UInt8(1),
                        enteredWords: enteredWords.flatMap(\.self)
                    )

                self.focusField = ImportFieldNumber(focusField)
            }
        }
        .onChange(of: focusField, initial: false, onChangeFocusField)
        .onChange(of: nfcReader.scannedMessage, initial: false, onChangeNfcMessage)
        .onChange(of: enteredWords) {
            if isAllWordsValid {
                focusField = nil
            }
        }
        .onDisappear {
            focusField = nil
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
                message:
                "The words you entered does not create a valid wallet. Please check the words and try again.",
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
            .ignoresSafeArea(.all)
        }
    }

    // MARK: OnChange Functions

    func onChangeFocusField(_: ImportFieldNumber?, _ new: ImportFieldNumber?) {
        filteredSuggestions = []

        // don't change tab when focus is cleared (e.g., after paste)
        guard let new else { return }

        let focusFieldNumber = new.fieldNumber
        if (focusFieldNumber % groupsOf) == 1 {
            withAnimation {
                tabIndex = Int(focusFieldNumber / groupsOf)
            }
        }
    }

    func onChangeNfcMessage(_: NfcMessage?, _ new: NfcMessage?) {
        guard new != nil else { return }

        // try string first
        if let string = new?.string() {
            do {
                let words = try groupedPlainWordsOf(mnemonic: string, groups: UInt8(groupsOf))
                return setWords(words)
            } catch {
                Log.error("Error NFC word parsing: \(error)")
            }
        }

        // if string doesn't work, try data
        if let data = new?.data() {
            do {
                let seedQR = try SeedQr.newFromData(data: data)
                let words = seedQR.groupedPlainWords(groupsOf: UInt8(groupsOf))
                return setWords(words)
            } catch {
                Log.error("Error NFC word parsing from data: \(error)")
            }
        }

        // both failed - show error
        app.alertState = TaggedItem(.general(
            title: "Invalid NFC Tag",
            message: "Please scan a valid seed phrase NFC tag"
        ))
    }

    func setWords(_ words: [[String]]) {
        let numberOfWords = words.compactMap(\.count).reduce(0, +)
        switch numberOfWords {
        case 12: self.numberOfWords = .twelve
        case 24: self.numberOfWords = .twentyFour
        default:
            Log.warn("Invalid number of words: \(numberOfWords)")
            scanError = TaggedString(
                "Invalid number of words: \(numberOfWords), we only support 12 or 24 words"
            )

            sheetState = .none
            return
        }

        // reset scanner and nfc reader on successful scan
        scanner.reset()
        nfcReader.resetReader()
        nfcReader.session = nil

        enteredWords = words
        sheetState = .none
        tabIndex = words.count - 1
    }

    func handlePasteMnemonic(_ mnemonicString: String) {
        // extract word-like tokens, stripping numbers and punctuation
        let words = mnemonicString
            .split(whereSeparator: { $0.isWhitespace })
            .map { String($0).lowercased() }
            .filter { word in
                // keep only alphabetic strings (filters out "1.", "2)", etc.)
                word.allSatisfy(\.isLetter)
            }

        // need 12 or 24 words
        guard words.count == 12 || words.count == 24 else {
            alertState = .init(.invalidWords)
            return
        }

        // group words into chunks of groupsOf (12)
        let grouped = stride(from: 0, to: words.count, by: groupsOf).map {
            Array(words[$0 ..< min($0 + groupsOf, words.count)])
        }
        setWords(grouped)

        // validate - show alert if invalid
        do {
            _ = try groupedPlainWordsOf(mnemonic: words.joined(separator: " "), groups: UInt8(groupsOf))
        } catch {
            Log.debug("Invalid pasted mnemonic: \(error)")
            alertState = .init(.invalidWords)
        }
    }
}

private struct KeyboardToolbarShapeModifier: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content
                .clipShape(Capsule())
                .padding(.horizontal, 8)
        } else {
            content
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
            .environment(AppManager.shared)
    }
}

#Preview("24 Words") {
    NavigationStack {
        HotWalletImportScreen(numberOfWords: .twentyFour)
            .environment(AppManager.shared)
    }
}
