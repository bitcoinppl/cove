//
//  VerifyWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

// MARK: CONTAINER

struct VerifyWordsContainer: View {
    @Environment(\.sizeCategory) var sizeCategory
    @Environment(AppManager.self) private var app

    let id: WalletId
    let onVerified: (() -> Void)?

    @State private var verificationComplete = false

    init(id: WalletId, onVerified: (() -> Void)? = nil) {
        self.id = id
        self.onVerified = onVerified
    }

    var body: some View {
        WalletManagerHost(walletId: id, loading: {
            Text("Loading....")
        }, onError: { error in
            Log.error("VerifyWords failed to initialize: \(error)")
            app.trySelectLatestOrNewWallet()
        }) { manager in
            VerifyWordsLoadedView(
                manager: manager,
                onVerified: onVerified,
                sizeCategory: sizeCategory,
                verificationComplete: $verificationComplete
            )
        }
        .navigationTitle("Verify Recovery Words")
        .navigationBarTitleDisplayMode(.inline)
        .toolbarColorScheme(.dark, for: .navigationBar)
    }
}

private struct VerifyWordsLoadedView: View {
    @Environment(AppManager.self) private var app

    let manager: WalletManager
    let onVerified: (() -> Void)?
    let sizeCategory: ContentSizeCategory

    @Binding var verificationComplete: Bool
    @State private var stateMachine: WordVerifyStateMachine?
    @State private var loadingError: Error?

    var body: some View {
        VerifyWordsLoadStateContent(
            stateMachine: stateMachine,
            loadingError: loadingError,
            manager: manager,
            onVerified: onVerified,
            verificationComplete: $verificationComplete,
            retry: retryLoading,
            returnToWallet: app.trySelectLatestOrNewWallet
        )
        .task(id: ObjectIdentifier(manager)) {
            await loadStateMachine()
        }
    }

    private func retryLoading() {
        Task {
            await loadStateMachine()
        }
    }

    @MainActor
    private func loadStateMachine() async {
        loadingError = nil

        do {
            let validator = try manager.wordValidator()
            verificationComplete = false
            stateMachine = WordVerifyStateMachine(validator: validator, startingWordNumber: 1)
        } catch {
            Log.error("VerifyWords failed to initialize: \(error)")
            stateMachine = nil
            loadingError = error
        }
    }
}

private struct VerifyWordsLoadStateContent: View {
    let stateMachine: WordVerifyStateMachine?
    let loadingError: Error?
    let manager: WalletManager
    let onVerified: (() -> Void)?
    @Binding var verificationComplete: Bool
    let retry: () -> Void
    let returnToWallet: () -> Void

    var body: some View {
        if let stateMachine {
            Group {
                if verificationComplete {
                    VerificationCompleteScreen(manager: manager, onVerified: onVerified)
                } else {
                    VerifyWordsScreen(
                        manager: manager,
                        stateMachine: stateMachine,
                        verificationComplete: $verificationComplete
                    )
                }
            }
            .transition(
                .asymmetric(
                    insertion: .move(edge: .trailing),
                    removal: .move(edge: .leading)
                )
            )
            .background(
                Color.midnightBlue
                    .ignoresSafeArea(.all)
            )
            .adaptiveToolbarStyle()
        } else if let loadingError {
            VerifyWordsLoadingErrorView(
                error: loadingError,
                retry: retry,
                returnToWallet: returnToWallet
            )
        } else {
            Text("Loading....")
        }
    }
}

private struct VerifyWordsLoadingErrorView: View {
    let error: Error
    let retry: () -> Void
    let returnToWallet: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Text("Unable to load recovery word verification")
                .font(.headline)
                .foregroundStyle(.white)
                .multilineTextAlignment(.center)

            Text(error.localizedDescription)
                .font(.footnote)
                .foregroundStyle(.coveLightGray.opacity(0.75))
                .multilineTextAlignment(.center)

            VStack(spacing: 12) {
                Button("Try Again", action: retry)
                    .buttonStyle(PrimaryButtonStyle())

                Button("Return to Wallet", action: returnToWallet)
                    .buttonStyle(DarkButtonStyle())
            }
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.midnightBlue.ignoresSafeArea())
    }
}

// MARK: Screen

struct VerifyWordsScreen: View {
    @Environment(\.sizeCategory) private var sizeCategory
    @Environment(\.navigate) private var navigate
    @Environment(AppManager.self) private var app

    // args
    let manager: WalletManager
    let stateMachine: WordVerifyStateMachine
    @Binding var verificationComplete: Bool

    // private
    @State private var checkState: WordCheckState = .none
    @State private var wordNumber: Int
    @State private var possibleWords: [String]
    @State private var incorrectGuesses = 0

    @Namespace private var namespace

    /// alerts
    private enum AlertType: Identifiable {
        case words, skip
        var id: Self {
            self
        }
    }

    @State private var activeAlert: AlertType?

    var id: WalletId {
        manager.walletMetadata.id
    }

    init(manager: WalletManager, stateMachine: WordVerifyStateMachine, verificationComplete: Binding<Bool>) {
        self.manager = manager
        self.stateMachine = stateMachine
        _verificationComplete = verificationComplete

        let wordNum = Int(stateMachine.wordNumber())
        wordNumber = wordNum
        possibleWords = stateMachine.possibleWords()
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

        let transition = stateMachine.selectWord(word: word)

        let animation = Animation.spring().speed(2.0)

        withAnimation(animation) {
            checkState = transition.newState
        } completion: {
            checkWord(word)
        }
    }

    @MainActor
    func deselectWord(_ animation: Animation = .spring(), completion: @escaping () -> Void = {}) {
        withAnimation(animation, completionCriteria: .logicallyComplete) {
            checkState = .returning(word: currentWord ?? "")
        } completion: {
            checkState = .none
            completion()
        }
    }

    @MainActor
    func checkWord(_: String) {
        let transition = stateMachine.animationComplete()

        guard case .correct = transition.newState else {
            handleIncorrectWord(transition: transition)
            return
        }

        withAnimation(Animation.spring().speed(3), completionCriteria: .logicallyComplete) {
            checkState = transition.newState
        } completion: {
            self.handleCorrectWordDwell()
        }
    }

    @MainActor
    private func handleCorrectWordDwell() {
        let dwellTransition = stateMachine.dwellComplete()
        checkState = .none

        guard dwellTransition.shouldAdvanceWord else { return }

        if stateMachine.isComplete() {
            withAnimation(.easeInOut(duration: 0.3)) {
                verificationComplete = true
            }
        } else {
            withAnimation(.spring().speed(3)) {
                wordNumber = Int(stateMachine.wordNumber())
                possibleWords = stateMachine.possibleWords()
            }
        }
    }

    @MainActor
    private func handleIncorrectWord(transition: StateTransition) {
        incorrectGuesses += 1
        withAnimation(Animation.spring().speed(2)) {
            checkState = transition.newState
        } completion: {
            _ = self.stateMachine.dwellComplete()
            self.deselectWord(.spring().speed(3)) {
                _ = self.stateMachine.returnComplete()
            }
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

    var currentWord: String? {
        switch checkState {
        case let .checking(word), let .correct(word), let .incorrect(word), let .returning(word):
            word
        case .none:
            nil
        }
    }

    var body: some View {
        VerifyWordsLayout(
            sizeCategory: sizeCategory,
            wordNumber: wordNumber,
            currentWord: currentWord,
            possibleWords: possibleWords,
            columns: columns,
            checkState: checkState,
            checkingWordColor: checkingWordColor,
            checkingWordBackground: checkingWordBg,
            isDisabled: isDisabled,
            isReturning: isReturning,
            namespace: namespace,
            matchedGeometryId: matchedGeoId,
            selectWord: selectWord,
            deselectWord: deselectCheckingWord,
            showWords: showWordsAlert,
            skipVerification: showSkipAlert
        )
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

    private func deselectCheckingWord() {
        guard case .checking = checkState else { return }

        deselectWord()
    }

    private func showWordsAlert() {
        activeAlert = .words
    }

    private func showSkipAlert() {
        activeAlert = .skip
    }

    private var isReturning: Bool {
        if case .returning = checkState { return true }
        return false
    }
}

private struct VerifyWordsLayout: View {
    let sizeCategory: ContentSizeCategory
    let wordNumber: Int
    let currentWord: String?
    let possibleWords: [String]
    let columns: [GridItem]
    let checkState: WordCheckState
    let checkingWordColor: Color
    let checkingWordBackground: Color
    let isDisabled: Bool
    let isReturning: Bool
    let namespace: Namespace.ID
    let matchedGeometryId: (String) -> String
    let selectWord: (String) -> Void
    let deselectWord: () -> Void
    let showWords: () -> Void
    let skipVerification: () -> Void

    var body: some View {
        GeometryReader { proxy in
            let scrollableLayout = usesCompactLayout(
                sizeCategory: sizeCategory,
                availableHeight: proxy.size.height
            )

            Group {
                if scrollableLayout {
                    VStack(spacing: 0) {
                        ScrollView {
                            VerifyWordsMainContent(
                                wordNumber: wordNumber,
                                currentWord: currentWord,
                                possibleWords: possibleWords,
                                columns: columns,
                                checkState: checkState,
                                checkingWordColor: checkingWordColor,
                                checkingWordBackground: checkingWordBackground,
                                isDisabled: isDisabled,
                                isReturning: isReturning,
                                namespace: namespace,
                                matchedGeometryId: matchedGeometryId,
                                selectWord: selectWord,
                                deselectWord: deselectWord,
                                usesFlexibleMiddleSpacer: false,
                                includesActions: false,
                                showWords: showWords,
                                skipVerification: skipVerification
                            )
                            .padding(.bottom, 24)
                        }
                        .scrollIndicators(.hidden)

                        VerifyWordsCompactActions(
                            showWords: showWords,
                            skipVerification: skipVerification
                        )
                    }
                    .frame(width: proxy.size.width, height: proxy.size.height)
                } else {
                    VerifyWordsMainContent(
                        wordNumber: wordNumber,
                        currentWord: currentWord,
                        possibleWords: possibleWords,
                        columns: columns,
                        checkState: checkState,
                        checkingWordColor: checkingWordColor,
                        checkingWordBackground: checkingWordBackground,
                        isDisabled: isDisabled,
                        isReturning: isReturning,
                        namespace: namespace,
                        matchedGeometryId: matchedGeometryId,
                        selectWord: selectWord,
                        deselectWord: deselectWord,
                        usesFlexibleMiddleSpacer: true,
                        includesActions: true,
                        showWords: showWords,
                        skipVerification: skipVerification
                    )
                    .frame(width: proxy.size.width, height: proxy.size.height)
                }
            }
        }
    }
}

private struct VerifyWordsMainContent: View {
    let wordNumber: Int
    let currentWord: String?
    let possibleWords: [String]
    let columns: [GridItem]
    let checkState: WordCheckState
    let checkingWordColor: Color
    let checkingWordBackground: Color
    let isDisabled: Bool
    let isReturning: Bool
    let namespace: Namespace.ID
    let matchedGeometryId: (String) -> String
    let selectWord: (String) -> Void
    let deselectWord: () -> Void
    let usesFlexibleMiddleSpacer: Bool
    let includesActions: Bool
    let showWords: () -> Void
    let skipVerification: () -> Void

    var body: some View {
        VStack(spacing: 24) {
            VerifyWordsSelection(
                wordNumber: wordNumber,
                currentWord: currentWord,
                possibleWords: possibleWords,
                columns: columns,
                checkState: checkState,
                checkingWordColor: checkingWordColor,
                checkingWordBackground: checkingWordBackground,
                isDisabled: isDisabled,
                isReturning: isReturning,
                namespace: namespace,
                matchedGeometryId: matchedGeometryId,
                selectWord: selectWord,
                deselectWord: deselectWord
            )

            if usesFlexibleMiddleSpacer {
                Spacer()
            }

            VerifyWordsIntroduction()

            if !isMiniDevice {
                Spacer()
            }

            if includesActions {
                Divider()
                    .overlay(.coveLightGray.opacity(0.50))

                VerifyWordsActionButtons(
                    showWords: showWords,
                    skipVerification: skipVerification
                )
                .safeAreaPadding(.bottom, 30)
            }
        }
        .padding()
    }
}

private struct VerifyWordsSelection: View {
    let wordNumber: Int
    let currentWord: String?
    let possibleWords: [String]
    let columns: [GridItem]
    let checkState: WordCheckState
    let checkingWordColor: Color
    let checkingWordBackground: Color
    let isDisabled: Bool
    let isReturning: Bool
    let namespace: Namespace.ID
    let matchedGeometryId: (String) -> String
    let selectWord: (String) -> Void
    let deselectWord: () -> Void

    var body: some View {
        VStack(spacing: 24) {
            Text("What is word #\(wordNumber)?")
                .foregroundStyle(.white)
                .font(.title2)
                .fontWeight(.semibold)

            VStack(spacing: 10) {
                if let checkingWord = currentWord {
                    Button(action: deselectWord) {
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
                            .background(checkingWordBackground)
                            .cornerRadius(10)
                    }
                    .matchedGeometryEffect(
                        id: matchedGeometryId(checkingWord),
                        in: namespace,
                        isSource: checkState != .none && !isReturning
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
                    .disabled(isDisabled || currentWord == word)
                    .contentShape(Rectangle())
                    .padding(.horizontal)
                    .padding(.vertical, 12)
                    .background(Color.btnPrimary)
                    .cornerRadius(10)
                    .matchedGeometryEffect(
                        id: matchedGeometryId(word),
                        in: namespace,
                        isSource: checkState == .none || isReturning
                    )
                    .opacity(currentWord == word ? 0 : 1)
                }
            }
            .padding(.vertical)
        }
    }
}

private struct VerifyWordsIntroduction: View {
    var body: some View {
        VStack(spacing: 24) {
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
        }
    }
}

private struct VerifyWordsActionButtons: View {
    let showWords: () -> Void
    let skipVerification: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Button(action: showWords) {
                Text("Show Words")
            }
            .buttonStyle(PrimaryButtonStyle())

            Button(action: skipVerification) {
                Text("Skip Verification")
                    .foregroundStyle(.white)
                    .font(.caption)
                    .fontWeight(.medium)
            }
        }
    }
}

private struct VerifyWordsCompactActions: View {
    let showWords: () -> Void
    let skipVerification: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Divider()
                .overlay(.coveLightGray.opacity(0.50))

            VerifyWordsActionButtons(
                showWords: showWords,
                skipVerification: skipVerification
            )
        }
        .padding(.horizontal)
        .padding(.top, 12)
        .padding(.bottom, 56)
        .background(Color.midnightBlue)
    }
}

#Preview {
    struct Container: View {
        @State var manager = WalletManager(preview: .only)
        @State var stateMachine: WordVerifyStateMachine

        init() {
            let validator = WordValidator.preview(preview: true)
            _stateMachine = State(initialValue: WordVerifyStateMachine(validator: validator, startingWordNumber: 1))
        }

        var body: some View {
            VerifyWordsScreen(
                manager: manager,
                stateMachine: stateMachine,
                verificationComplete: Binding.constant(false)
            )
            .environment(AppManager.shared)
            .environment(AuthManager.shared)
        }
    }

    return
        NavigationStack {
            AsyncPreview {
                Container()
            }
        }
}
