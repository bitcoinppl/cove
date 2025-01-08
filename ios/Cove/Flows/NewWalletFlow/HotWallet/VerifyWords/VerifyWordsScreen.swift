//
//  VerifyWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

// MARK: CONTAINER

struct VerifyWordsContainer: View {
    @Environment(AppManager.self) private var app
    let id: WalletId

    @State var verificationComplete = false
    @State private var manager: WalletManager? = nil
    @State private var validator: WordValidator? = nil

    func initOnAppear() {
        do {
            let manager = try app.getWalletManager(id: id)
            let validator = try manager.rust.wordValidator()

            self.manager = manager
            self.validator = validator
        } catch {
            Log.error("VerifyWords failed to initialize: \(error)")
        }
    }

    @ViewBuilder
    func LoadedScreen(manager: WalletManager, validator: WordValidator) -> some View {
        if verificationComplete {
            VerificationCompleteScreen(manager: manager)
                .transition(
                    .asymmetric(
                        insertion: .move(edge: .trailing),
                        removal: .move(edge: .leading)
                    ))
        } else {
            VerifyWordsScreen(
                manager: manager,
                validator: validator,
                verificationComplete: $verificationComplete
            )
            .transition(
                .asymmetric(
                    insertion: .move(edge: .trailing),
                    removal: .move(edge: .leading)
                ))
        }
    }

    var body: some View {
        Group {
            if let manager, let validator {
                LoadedScreen(manager: manager, validator: validator)
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
    @Environment(AppManager.self) private var app

    // args
    let manager: WalletManager
    let validator: WordValidator
    @Binding var verificationComplete: Bool

    // private
    @State private var wordNumber: Int
    @State private var possibleWords: [String]
    @State private var checkState: CheckState = .none
    @State private var incorrectGuesses = 0

    @Namespace private var namespace

    // alerts
    private enum AlertType: Identifiable {
        case words, skip
        var id: Self { self }
    }

    @State private var activeAlert: AlertType?

    var id: WalletId {
        manager.walletMetadata.id
    }

    init(manager: WalletManager, validator: WordValidator, verificationComplete: Binding<Bool>) {
        self.manager = manager
        self.validator = validator
        _verificationComplete = verificationComplete

        wordNumber = 1

        possibleWords = validator.possibleWords(for: 1)
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

    @MainActor
    func selectWord(_ word: String) {
        // if already checking, skip
        if checkState != .none {
            withAnimation(.spring().speed(6)) { checkState = .none }
            return
        }

        let animation =
            if validator.isWordCorrect(word: word, for: UInt8(wordNumber)) {
                Animation.spring().speed(2.25)
            } else {
                Animation.spring().speed(1.75)
            }

        withAnimation(animation) {
            checkState = .checking(word)
        } completion: {
            checkWord(word)
        }
    }

    @MainActor
    func deselectWord(_ animation: Animation = .spring(), completion: @escaping () -> Void = {}) {
        withAnimation(animation, completionCriteria: .logicallyComplete) {
            checkState = .returning
        } completion: {
            checkState = .none
            completion()
        }
    }

    @MainActor
    func checkWord(_ word: String) {
        if validator.isWordCorrect(word: word, for: UInt8(wordNumber)) {
            withAnimation(Animation.spring().speed(3), completionCriteria: .removed) {
                checkState = .correct(word)
            } completion: {
                nextWord()
            }
        } else {
            withAnimation(Animation.spring().speed(2)) {
                checkState = .incorrect(word)
            } completion: {
                deselectWord(
                    .spring().speed(3),
                    completion: {
                        incorrectGuesses += 1
                    }
                )
            }
        }
    }

    @MainActor
    func nextWord() {
        if validator.isComplete(wordNumber: UInt8(wordNumber)) {
            withAnimation(.easeInOut(duration: 0.3)) {
                verificationComplete = true
            } completion: {
                checkState = .none
            }
            return
        }

        withAnimation(.spring().speed(3)) {
            wordNumber += 1
            possibleWords = validator.possibleWords(for: UInt8(wordNumber))
        } completion: {
            checkState = .none
        }
    }

    func matchedGeoId(for word: String) -> String {
        "\(wordNumber)-\(word)-\(incorrectGuesses)"
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
        checkState != .none
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
                    Button(action: {
                        guard case .checking = checkState else { return }
                        deselectWord()
                    }) {
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
                    .matchedGeometryEffect(
                        id: matchedGeoId(for: checkingWord),
                        in: namespace,
                        isSource: checkState != .none
                    )
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
                            .matchedGeometryEffect(
                                id: matchedGeoId(for: word),
                                in: namespace,
                                isSource: checkState == .none
                            )
                        } else {
                            Text(word).opacity(0)
                        }
                    }
                }
            }
            .padding(.vertical)

            Spacer()

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
                    Text(
                        "To confirm that you've securely saved your recovery phrase, please select the correct word"
                    )
                    .font(.footnote)
                    .foregroundStyle(.coveLightGray.opacity(0.75))
                    .fixedSize(horizontal: false, vertical: true)

                    Spacer()
                }
            }

            if !isMiniDevice { Spacer() }

            Divider()
                .overlay(.coveLightGray.opacity(0.50))

            VStack(spacing: 16) {
                Button(action: { activeAlert = .words }) {
                    Text("Show Words")
                }
                .buttonStyle(PrimaryButtonStyle())

                Button(action: { activeAlert = .skip }) {
                    Text("Skip Verification")
                        .foregroundStyle(.white)
                        .font(.caption)
                        .fontWeight(.medium)
                }
            }
            // mini and se only
            .safeAreaPadding(.bottom, isMiniDevice ? 30 : 0)
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
    case none
    case checking(String)
    case correct(String)
    case incorrect(String)
    case returning

    var word: String? {
        switch self {
        case let .checking(word):
            word
        case let .correct(word):
            word
        case let .incorrect(word):
            word
        case .none, .returning:
            nil
        }
    }
}

#Preview {
    struct Container: View {
        @State var manager = WalletManager(preview: "preview_only")
        @State var validator = WordValidator.preview(preview: true)

        var body: some View {
            VerifyWordsScreen(
                manager: manager,
                validator: validator,
                verificationComplete: Binding.constant(false)
            )
            .environment(AppManager())
            .environment(AuthManager())
        }
    }

    return
        NavigationStack {
            AsyncPreview {
                Container()
            }
        }
}
