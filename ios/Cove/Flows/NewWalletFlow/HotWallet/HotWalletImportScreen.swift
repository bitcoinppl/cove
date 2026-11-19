//
//  HotWalletImportScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

private let groupsOf = HotWalletImportScreen.GROUPS_OF

/// Alerts the import screen raises itself
///
/// Onboarding shows this screen outside the app's main view, which presents the app's own alert
/// state, so everything the screen reports goes through these
enum HotWalletImportAlertState: Equatable {
    case invalidWords
    case duplicateWallet(WalletId)
    case scan(HotWalletImportScanAlert)
    case importError(String)
}

/// Why a QR or NFC scan couldn't fill in the words
enum HotWalletImportScanAlert: Equatable {
    case invalidSeedQr
    case invalidNfcTag
    case unsupportedWordCount(Int)
    case noCameraPermission
}

enum HotWalletImportSheetState: Equatable {
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
    @State private var numberOfWords: NumberOfBip39Words
    let importType: ImportType
    let autoImportScannedWords: Bool
    let onImported: ((WalletId) -> Void)?

    init(
        numberOfWords: NumberOfBip39Words,
        importType: ImportType = .manual,
        autoImportScannedWords: Bool = false,
        onImported: ((WalletId) -> Void)? = nil
    ) {
        _numberOfWords = State(initialValue: numberOfWords)
        self.importType = importType
        self.autoImportScannedWords = autoImportScannedWords
        self.onImported = onImported
    }

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

    @State var manager: ImportWalletManager = .init()

    @State var enteredWords: [[String]] = [[]]
    @State var filteredSuggestions: [String] = []

    // alerts & sheets
    @State private var alertState: TaggedItem<HotWalletImportAlertState>? = .none
    @State private var sheetState: TaggedItem<HotWalletImportSheetState>? = .none

    /// qr code scanning
    @State private var scanner = QrScanner()

    // nfc scanning
    @State private var nfcReader: NFCReader = .init()
    @State private var tasks: [Task<Void, any Error>] = []

    private var presentationContext: HotWalletImportPresentationContext {
        HotWalletImportPresentationContext(
            app: app,
            alertState: $alertState,
            handleScan: handleScan,
            onImported: onImported
        )
    }

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
        // the camera can still deliver a frame while the closed sheet animates away, which would
        // start a multi-part scan again after the close reset it
        guard sheetState != nil else { return }

        switch result {
        case let .failure(error):
            handleScanFailure(error)
        case let .success(scanResult):
            handleScannedFrame(StringOrData(scanResult.data))
        }
    }

    /// The scanner reports a failure once, when the camera can't start; it says so itself unless
    /// access is denied, which closes its sheet and explains how to allow it
    private func handleScanFailure(_ error: ScanError) {
        guard case .permissionDenied = error else {
            Log.error("camera failed to start: \(error)")
            return
        }

        Log.info("camera permission denied")
        sheetState = .none
        alertState = .init(.scan(.noCameraPermission))
    }

    private func handleScannedFrame(_ qr: StringOrData) {
        do {
            switch try scanner.scan(qr: qr) {
            case let .complete(multiFormat, haptic):
                haptic.trigger()
                scanner.reset()
                handleScannedFormat(multiFormat)

            case let .inProgress(_, haptic):
                // multi-part QR in progress - keep scanning
                haptic.trigger()
            }
        } catch let error as MultiQrError {
            // a frame from another QR, or one the camera misread, leaves the scan going
            if case .RecoverableFrame = error { return }

            handleUnreadableSeedQr(error)
        } catch {
            handleUnreadableSeedQr(error)
        }
    }

    private func handleScannedFormat(_ multiFormat: MultiFormat) {
        guard case let .mnemonic(mnemonic) = multiFormat else {
            sheetState = .none
            alertState = .init(.scan(.invalidSeedQr))
            return
        }

        let words: [[String]]
        do {
            words = try groupedPlainWordsOf(
                mnemonic: mnemonic.words().joined(separator: " "),
                groups: UInt8(groupsOf)
            )
        } catch {
            handleUnreadableSeedQr(error)
            return
        }

        guard setWords(words) else { return }

        if autoImportScannedWords {
            importWallet(enteredWords: words)
        }
    }

    private func handleUnreadableSeedQr(_ error: Error) {
        Log.error("Seed QR failed to scan: \(error.localizedDescription)")
        scanner.reset()
        sheetState = .none
        alertState = .init(.scan(.invalidSeedQr))
    }

    func importWallet(enteredWords wordsToImport: [[String]]? = nil) {
        do {
            let walletMetadata = try manager.rust.importWallet(enteredWords: wordsToImport ?? enteredWords)
            if let onImported {
                onImported(walletMetadata.id)
                return
            }

            try app.selectWalletOrThrow(walletMetadata.id)
            app.clearWalletManager()
            app.resetRoute(to: .selectedWallet(walletMetadata.id))
        } catch let error as ImportWalletError {
            switch error {
            case let .InvalidWordGroup(error):
                Log.debug("Invalid words: \(error)")
                alertState = .init(.invalidWords)
            case let .WalletAlreadyExists(walletId):
                Log.warn("Attempted to import words for an existing hot wallet: \(walletId)")
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
            case .WalletIdentityCollision:
                Log.error("Multiple existing wallets match the imported secret")
                alertState = .init(.importError(
                    "More than one existing wallet matches these words, so Cove can't tell which one to update. Remove the duplicate wallet and try again."
                ))
            case let .WalletIdentity(message):
                Log.error("Unable to match imported words to an existing wallet: \(message)")
                alertState = .init(.importError(
                    "Cove couldn't safely match these words to an existing wallet: \(message)"
                ))
            case let .UpgradeRollback(message):
                Log.error("Failed to roll back a partial wallet upgrade: \(message)")
                alertState = .init(.importError(
                    "The import failed partway and Cove couldn't fully undo it. Restart Cove and check this wallet before trying again."
                ))
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

    @ToolbarContentBuilder
    var ToolbarContent: some ToolbarContent {
        ToolbarItemGroup(placement: .topBarTrailing) {
            HStack(spacing: 5) {
                Button(action: nfcReader.scan) {
                    Image(systemName: "wave.3.right")
                        .font(.subheadline)
                        .foregroundColor(.white)
                }
                .accessibilityLabel("Scan NFC Tag")
                .accessibilityIdentifier("hotWalletImport.scanNfc")

                Button(action: { sheetState = .init(.qrCode) }) {
                    Image(systemName: "qrcode.viewfinder")
                        .font(.subheadline)
                        .foregroundColor(.white)
                }
                .accessibilityLabel("Scan QR Code")
                .accessibilityIdentifier("hotWalletImport.scanQr")
            }
        }
    }

    var body: some View {
        HotWalletImportMainContent(
            keyboardIsShowing: keyboardObserver.keyboardIsShowing,
            isCompactLayout: usesCompactTypography(sizeCategory: sizeCategory),
            numberOfWords: numberOfWords,
            tabIndex: $tabIndex,
            enteredWords: $enteredWords,
            filteredSuggestions: $filteredSuggestions,
            focusField: $focusField,
            onPasteMnemonic: handlePasteMnemonic,
            importWallet: { importWallet() }
        )
        .background {
            KeyboardAccessoryHost(
                isVisible: keyboardObserver.keyboardIsShowing && focusField != nil
                    && !filteredSuggestions.isEmpty,
                height: accessoryHeight
            ) {
                HotWalletImportKeyboardToolbar(
                    filteredSuggestions: filteredSuggestions,
                    accessoryHeight: accessoryHeight,
                    selectWord: selectWordInKeyboard
                )
            }
            .frame(width: 0, height: 0)
        }
        .animation(.easeInOut(duration: 0.25), value: keyboardObserver.keyboardIsShowing)
        .padding()
        .padding(.bottom, 40)
        .toolbar { ToolbarContent }
        .navigationTitle("Import Wallet")
        .navigationBarTitleDisplayMode(.inline)
        .toolbarColorScheme(.dark, for: .navigationBar)
        .presentingSheetAndAlert(
            sheet: $sheetState,
            sheetContext: presentationContext,
            alert: $alertState,
            alertContext: presentationContext,
            defaultTitle: "Error"
        )
        .modifier(ConditionalTintModifier())
        .onAppear(perform: initOnAppear)
        .onChange(of: sheetState, initial: true, handleSheetStateChange)
        .onChange(of: focusField, initial: false, onChangeFocusField)
        .onChange(of: nfcReader.scannedMessage, initial: false, onChangeNfcMessage)
        .onChange(of: enteredWords, handleEnteredWordsChange)
        .onDisappear(perform: handleDisappear)
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

    // MARK: OnChange Functions

    private func handleSheetStateChange(
        _ oldState: TaggedItem<HotWalletImportSheetState>?,
        _ newState: TaggedItem<HotWalletImportSheetState>?
    ) {
        guard oldState != nil, newState == nil else { return }

        // a multi-part QR left half scanned would hold the next scan back waiting for its other
        // parts
        scanner.reset()

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

    private func handleEnteredWordsChange(_: [[String]], _: [[String]]) {
        if isAllWordsValid {
            focusField = nil
        }
    }

    private func handleDisappear() {
        focusField = nil
        nfcReader.resetReader()
        nfcReader.session = nil
        for task in tasks {
            task.cancel()
        }
    }

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
                setWords(words)
                return
            } catch {
                Log.error("Error NFC word parsing: \(error)")
            }
        }

        // if string doesn't work, try data
        if let data = new?.data() {
            do {
                let seedQR = try SeedQr.newFromData(data: data)
                let words = seedQR.groupedPlainWords(groupsOf: UInt8(groupsOf))
                setWords(words)
                return
            } catch {
                Log.error("Error NFC word parsing from data: \(error)")
            }
        }

        // both failed - show error
        alertState = .init(.scan(.invalidNfcTag))
    }

    /// Fills the screen with scanned or pasted words, or closes the scanner and says why when
    /// there are neither 12 nor 24 of them, returning whether the words were taken
    @discardableResult
    func setWords(_ words: [[String]]) -> Bool {
        let wordCount = words.map(\.count).reduce(0, +)
        switch wordCount {
        case 12: numberOfWords = .twelve
        case 24: numberOfWords = .twentyFour
        default:
            Log.warn("Invalid number of words: \(wordCount)")
            sheetState = .none
            alertState = .init(.scan(.unsupportedWordCount(wordCount)))
            return false
        }

        // reset scanner and nfc reader on successful scan
        scanner.reset()
        nfcReader.resetReader()
        nfcReader.session = nil

        enteredWords = words
        sheetState = .none
        tabIndex = words.count - 1
        return true
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

struct KeyboardToolbarShapeModifier: ViewModifier {
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
