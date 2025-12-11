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
    @Environment(\.sizeCategory) var sizeCategory

    let id: WalletId

    @State var verificationComplete = false
    @State private var manager: WalletManager? = nil
    @State private var stateMachine: WordVerifyStateMachine? = nil

    func initOnAppear() {
        do {
            let manager = try app.getWalletManager(id: id)
            let validator = try manager.rust.wordValidator()
            let sm = WordVerifyStateMachine(validator: validator, startingWordNumber: 1)

            self.manager = manager
            self.stateMachine = sm
        } catch {
            Log.error("VerifyWords failed to initialize: \(error)")
        }
    }

    @ViewBuilder
    func LoadedScreen(manager: WalletManager, stateMachine: WordVerifyStateMachine) -> some View {
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
                stateMachine: stateMachine,
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
            if let manager, let stateMachine {
                if sizeCategory > .extraExtraExtraLarge || isMiniDevice {
                    ScrollView {
                        LoadedScreen(manager: manager, stateMachine: stateMachine)
                            .frame(minHeight: screenHeight, maxHeight: .infinity)
                    }
                    .background(
                        Color.midnightBlue
                            .ignoresSafeArea(.all)
                    )
                    .adaptiveToolbarStyle()
                } else {
                    LoadedScreen(manager: manager, stateMachine: stateMachine)
                }
            } else {
                Text("Loading....")
                    .onAppear(perform: initOnAppear)
            }
        }
        .toolbar {
            if sizeCategory < .extraExtraLarge || sizeCategory > .extraExtraExtraLarge,
               !isMiniDevice
            {
                ToolbarItem(placement: .principal) {
                    Text("Verify Recovery Words")
                        .foregroundStyle(.white)
                        .font(.callout)
                        .fontWeight(.semibold)
                }
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
    let stateMachine: WordVerifyStateMachine
    @Binding var verificationComplete: Bool

    // private
    @State private var checkState: WordCheckState = .none
    @State private var wordNumber: Int
    @State private var possibleWords: [String]
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
        VStack(spacing: 24) {
            Text("What is word #\(wordNumber)?")
                .foregroundStyle(.white)
                .font(.title2)
                .fontWeight(.semibold)

            VStack(spacing: 10) {
                if let checkingWord = currentWord {
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
                        id: matchedGeoId(for: word),
                        in: namespace,
                        isSource: checkState == .none || isReturning
                    )
                    .opacity(currentWord == word ? 0 : 1)
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

    private var isReturning: Bool {
        if case .returning = checkState { return true }
        return false
    }
}

#Preview {
    struct Container: View {
        @State var manager = WalletManager(preview: "preview_only")
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
