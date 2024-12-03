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
        if let model, let validator {
            VerifyWordsScreen(model: model, validator: validator)
        } else {
            Text("Loading....")
                .onAppear(perform: initOnAppear)
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

    var columns: [GridItem] {
        let item = GridItem(.adaptive(minimum: screenWidth * 0.25 - 20))
        return [item, item, item, item]
    }

    var body: some View {
        VStack(spacing: 48) {
            Spacer()
            Text("What is word #\(wordNumber)?")
                .foregroundStyle(.white)
                .font(.title2)
                .fontWeight(.semibold)

            Rectangle().frame(width: 200, height: 1)
                .foregroundColor(.white)

            LazyVGrid(columns: columns, spacing: 20) {
                ForEach(Array(possibleWords.enumerated()), id: \.offset) { _, word in
                    Button(action: {}) {
                        Text(word)
                            .font(.caption)
                            .foregroundStyle(.midnightBlue.opacity(0.90))
                            .multilineTextAlignment(.center)
                            .frame(alignment: .leading)
                            .minimumScaleFactor(0.90)
                            .lineLimit(1)
                    }
                    .padding(.horizontal)
                    .padding(.vertical, 12)
                    .background(Color.btnPrimary)
                    .cornerRadius(10)
                }
            }

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

                    Spacer()
                }

                Text("Your secret recovery words are the only way to recover your wallet if you lose your phone or switch to a different wallet. Once you leave this screen, you won’t be able to view them again.")
                    .font(.subheadline)
                    .foregroundStyle(.lightGray)
                    .opacity(0.75)
            }
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
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
        AsyncPreview {
            Container()
        }
}
