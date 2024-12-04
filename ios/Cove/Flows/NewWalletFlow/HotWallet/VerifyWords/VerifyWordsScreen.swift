//
//  VerifyWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

// MARK: CONTAINER

struct VerifyWordsContainer: View {
    @Environment(MainViewModel.self) private var app
    let id: WalletId

    @State private var verificationComplete = false
    @State private var model: WalletViewModel? = nil
    @State private var validator: WordValidator? = nil

    func initOnAppear() {
        do {
            let model = try app.getWalletViewModel(id: id)
            let validator = try model.rust.wordValidator()

            self.model = model
            self.validator = validator
        } catch {
            Log.error("VerifyWords failed to initialize: \(error)")
        }
    }

    var body: some View {
        Group {
            if let model, let validator {
                if verificationComplete {
                    VerificationCompleteScreen(model: model)
                } else {
                    VerifyWordsScreen(model: model, validator: validator)
                }
            } else {
                Text("Loading....")
                    .onAppear(perform: initOnAppear)
            }
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text("Verify Recovery Words")
                    .foregroundStyle(.white)
                    .font(.callout)
                    .fontWeight(.semibold)
            }
        }
    }
}

// MARK: Screen

struct VerifyWordsScreen: View {
    @Environment(\.navigate) private var navigate
    @Environment(MainViewModel.self) private var app

    // args
    let model: WalletViewModel
    let validator: WordValidator

    // private
    @State private var wordNumber: Int
    @State private var possibleWords: [String]
    @State private var checkState: CheckState = .none

    @State private var clicks = 0
    @State private var inSelectionProgress = false

    @Namespace private var namespace

    // alerts
    private enum AlertType: Identifiable {
        case words, skip
        var id: Self { self }
    }

    @State private var activeAlert: AlertType?

    var id: WalletId {
        model.walletMetadata.id
    }

    init(model: WalletViewModel, validator: WordValidator) {
        self.model = model
        self.validator = validator
        wordNumber = 1

        possibleWords = validator.possibleWords(for: 1)
    }

    var buttonIsDisabled: Bool {
        true
    }

    private func DisplayAlert(for alertType: AlertType) -> Alert {
        switch alertType {
        case .words:
            Alert(
                title: Text("See Secret Words?"),
                message: Text(
                    "Whoever has your secret words has access to your bitcoin. Please keep these safe and don't show them to anyone else."
                ),
                primaryButton: .destructive(Text("Yes, Show Me")) {
                    app.pushRoute(Route.secretWords(id))
                },
                secondaryButton: .cancel(Text("Cancel"))
            )
        case .skip:
            Alert(
                title: Text("Skip verifying words?"),
                message: Text(
                    "Are you sure you want to skip verifying words? Without having a back of these words, you could lose your bitcoin"
                ),
                primaryButton: .destructive(Text("Yes, Verify Later")) {
                    Log.debug("Skipping verification, going to wallet id: \(id)")
                    app.resetRoute(to: Route.selectedWallet(id))
                },
                secondaryButton: .cancel(Text("Cancel"))
            )
        }
    }

    func confirm(_ model: WalletViewModel, _: WordValidator) {
        do {
            try model.rust.markWalletAsVerified()
            app.resetRoute(to: Route.selectedWallet(id))
        } catch {
            Log.error("Error marking wallet as verified: \(error)")
        }
    }

    @MainActor
    func selectWord(_ word: String) {
        // if in the middle of a correct check, ignore
        if case .correct = checkState { return }
        if case .checking = checkState { return }

        if checkState == .none {
            let animation = if validator.isWordCorrect(word: word, for: UInt8(wordNumber)) {
                Animation.spring().speed(2.5)
            } else {
                Animation.spring().speed(1.5)
            }

            withAnimation(animation) {
                checkState = .checking(word)
            } completion: {
                checkWord(word)
            }
            return
        }

        // if already in the middle of another selection ignore
        if inSelectionProgress { return }

        inSelectionProgress = true
        withAnimation(.spring().speed(5), completionCriteria: .removed) {
            checkState = .none
        } completion: {
            selectWord(word)
        }
    }

    @MainActor
    func deselectWord(_ animation: Animation = .spring(), completion: @escaping () -> Void = {}) {
        withAnimation(animation) {
            checkState = .none
        } completion: {
            inSelectionProgress = false
            clicks += 1
            completion()
        }
    }

    @MainActor
    func checkWord(_ word: String) {
        if validator.isWordCorrect(word: word, for: UInt8(wordNumber)) {
            withAnimation(Animation.spring().speed(2))
                { checkState = .correct(word) }
                completion: { nextWord() }
        } else {
            withAnimation(Animation.spring().speed(1.25))
                { checkState = .incorrect(word) }
                completion: { deselectWord(.spring().speed(2)) }
        }
    }

    @MainActor
    func nextWord() {
        inSelectionProgress = false
        clicks += 1

        if validator.isComplete(wordNumber: UInt8(wordNumber)) {
            return confirm(model, validator)
        }

        withAnimation(.spring().speed(3)) {
            wordNumber += 1
            possibleWords = validator.possibleWords(for: UInt8(wordNumber))
        } completion: {
            deselectWord(.spring().speed(2.5))
        }
    }

    func matchedGeoId(for word: String) -> String {
        "\(wordNumber)-\(word)-\(clicks)"
    }

    var checkingWordBg: Color {
        switch checkState {
        case .correct:
            .green
        case .incorrect:
            .red
        default:
            .btnPrimary
        }
    }

    var checkingWordColor: Color {
        switch checkState {
        case .correct, .incorrect:
            Color.white
        default:
            Color.midnightBlue.opacity(0.90)
        }
    }

    var isDisabled: Bool {
        if inSelectionProgress, checkState != .none {
            return true
        }

        return false
    }

    var columns: [GridItem] {
        let item = GridItem(.adaptive(minimum: screenWidth * 0.25 - 20))
        return [item, item, item, item]
    }

    var body: some View {
        VStack(spacing: 24) {
            Text("What is word #\(wordNumber)?")
                .foregroundStyle(.white)
                .font(.title2)
                .fontWeight(.semibold)

            VStack(spacing: 10) {
                if let checkingWord = checkState.word {
                    Button(action: { deselectWord() }) {
                        Text(checkingWord)
                            .font(.caption)
                            .fontWeight(.medium)
                            .foregroundStyle(checkingWordColor)
                            .multilineTextAlignment(.center)
                            .frame(alignment: .leading)
                            .minimumScaleFactor(0.90)
                            .lineLimit(1)
                            .padding(.horizontal)
                            .padding(.vertical, 12)
                            .background(checkingWordBg)
                            .cornerRadius(10)
                    }
                    .matchedGeometryEffect(id: matchedGeoId(for: checkingWord), in: namespace)
                } else {
                    // take up the same space
                    Text("")
                        .padding(.vertical, 12)
                }

                Rectangle().frame(width: 200, height: 1)
                    .foregroundColor(.white)
            }

            LazyVGrid(columns: columns, spacing: 20) {
                ForEach(Array(possibleWords.enumerated()), id: \.offset) { _, word in
                    Group {
                        if checkState.word ?? "" != word {
                            Button(action: { selectWord(word) }) {
                                Text(word)
                                    .font(.caption)
                                    .foregroundStyle(.midnightBlue.opacity(0.90))
                                    .multilineTextAlignment(.center)
                                    .frame(alignment: .leading)
                                    .minimumScaleFactor(0.50)
                                    .lineLimit(1)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            .disabled(isDisabled)
                            .contentShape(Rectangle())
                            .padding(.horizontal)
                            .padding(.vertical, 12)
                            .background(Color.btnPrimary)
                            .cornerRadius(10)
                            .matchedGeometryEffect(id: matchedGeoId(for: word), in: namespace)
                        } else {
                            Text(word).opacity(0)
                        }
                    }
                }
            }
            .padding(.vertical)

            if !isMiniDevice { Spacer() }

            HStack {
                DotMenuView(selected: 3, size: 5)
                Spacer()
            }

            VStack(spacing: 12) {
                HStack {
                    Text("Verify your recovery words")
                        .font(.system(size: 38, weight: .semibold))
                        .foregroundColor(.white)
                        .fixedSize(horizontal: false, vertical: true)

                    Spacer()
                }

                HStack {
                    Text("To confirm that you've securely saved your recovery phrase, please drag and drop the word into their correct positions.")
                        .font(.footnote)
                        .foregroundStyle(.lightGray)
                        .opacity(0.75)
                        .fixedSize(horizontal: false, vertical: true)

                    Spacer()
                }
            }

            if !isMiniDevice { Spacer() }

            Divider()
                .overlay(.lightGray.opacity(0.50))

            VStack(spacing: 16) {
                Button(action: { activeAlert = .words }) {
                    Text("Show Words")
                        .font(.footnote)
                        .fontWeight(.medium)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 20)
                        .padding(.horizontal, 10)
                        .background(Color.btnPrimary)
                        .foregroundColor(.midnightBlue)
                        .cornerRadius(10)
                }

                Button(action: { activeAlert = .skip }) {
                    Text("Skip Verification")
                        .foregroundStyle(.white)
                        .font(.caption)
                        .fontWeight(.medium)
                }
            }
            // mini and se only
            .safeAreaPadding(.bottom, isMiniDevice ? 20 : 0)
        }
        .padding()
        .alert(item: $activeAlert) { alertType in
            DisplayAlert(for: alertType)
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
    }
}

enum CheckState: Equatable {
    case none, checking(String), correct(String), incorrect(String)

    var word: String? {
        switch self {
        case .checking(let word):
            word
        case .correct(let word):
            word
        case .incorrect(let word):
            word
        case .none:
            nil
        }
    }
}

#Preview {
    struct Container: View {
        @State var model = WalletViewModel(preview: "preview_only")
        @State var validator = WordValidator.preview(preview: true)

        var body: some View {
            VerifyWordsScreen(model: model, validator: validator)
                .environment(MainViewModel())
        }
    }

    return
        NavigationStack {
            AsyncPreview {
                Container()
            }
        }
}
