//
//  HotWalletCreateScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

struct HotWalletCreateScreen: View {
    @State private var model: PendingWalletViewModel

    init(numberOfWords: NumberOfBip39Words) {
        model = PendingWalletViewModel(numberOfWords: numberOfWords)
    }

    var body: some View {
        WordsView(model: model, groupedWords: model.rust.bip39WordsGrouped())
    }
}

let columns = [
    GridItem(.flexible()),
    GridItem(.flexible()),
    GridItem(.flexible()),
]

struct WordsView: View {
    var model: PendingWalletViewModel
    var groupedWords: [[GroupedWord]]
    @State private var tabIndex = 0
    @State private var showConfirmationAlert = false
    @Environment(\.dismiss) private var dismiss
    @Environment(\.navigate) private var navigate

    var lastIndex: Int {
        groupedWords.count - 1
    }

    var body: some View {
        VStack(spacing: 24) {
            StyledWordCard(tabIndex: $tabIndex) {
                ForEach(Array(groupedWords.enumerated()), id: \.offset) { index, wordGroup in
                    WordCardView(words: wordGroup).tag(index)
                }
            }
            .frame(height: screenHeight * 0.50)

            Spacer()

            HStack {
                DotMenuView(selected: 2, size: 5)
                Spacer()
            }

            HStack {
                Text("Recovery Words")
                    .font(.system(size: 38, weight: .semibold))
                    .lineSpacing(1.2)
                    .foregroundColor(.white)

                Spacer()
            }

            Text("Your secret recovery words are the only way to recover your wallet if you lose your phone or switch to a different wallet. Once you leave this screen, you won’t be able to view them again.")
                .font(.subheadline)
                .foregroundStyle(.lightGray)
                .opacity(0.70)

            HStack {
                Text("Please save these words in a secure location.")
                    .font(.subheadline)
                    .multilineTextAlignment(.leading)
                    .fontWeight(.bold)
                Spacer()
            }

            Divider()
                .overlay(.lightGray.opacity(0.50))

            VStack(spacing: 14) {
                Group {
                    if tabIndex == lastIndex {
                        Button("Save Wallet") {
                            do {
                                // save the wallet
                                let walletId = try model.rust.saveWallet().id

                                navigate(
                                    HotWalletRoute.verifyWords(walletId).intoRoute()
                                )
                            } catch {
                                // TODO: handle, maybe show an alert?
                                Log.error("Error \(error)")
                            }
                        }
                    } else {
                        Button("Next") {
                            withAnimation {
                                tabIndex += 1
                            }
                        }
                    }
                }
                .font(.subheadline)
                .fontWeight(.medium)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 20)
                .padding(.horizontal, 10)
                .background(Color.btnPrimary)
                .foregroundColor(.midnightBlue)
                .cornerRadius(10)
            }
        }
        .padding()
        .navigationBarTitleDisplayMode(.inline)
        .frame(maxHeight: .infinity)
        .background(
            Image(.newWalletPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: screenHeight * 0.75, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .opacity(0.5)
        )
        .background(Color.midnightBlue)
        .toolbar {
            ToolbarItem(placement: .navigationBarLeading) {
                Button(action: {
                    showConfirmationAlert = true
                }) {
                    HStack {
                        Image(systemName: "chevron.left")
                        Text("Back")
                    }
                    .foregroundStyle(.white)
                }
            }

            ToolbarItem(placement: .principal) {
                Text("Backup your wallet")
                    .fontWeight(.semibold)
                    .foregroundStyle(.white)
            }
        }
        .alert(isPresented: $showConfirmationAlert) {
            Alert(
                title: Text("⚠️ Wallet Not Saved ⚠️"),
                message: Text("You will have to write down a new set of words."),
                primaryButton: .destructive(Text("Yes, Go Back")) {
                    dismiss()
                },
                secondaryButton: .cancel(Text("Cancel"))
            )
        }
    }
}

struct WordCardView: View {
    let words: [GroupedWord]

    var body: some View {
        LazyVGrid(columns: columns, spacing: 20) {
            ForEach(words, id: \.self) { group in
                HStack(spacing: 0) {
                    Text("\(String(format: "%d", group.number)). ")
                        .foregroundColor(.black.opacity(0.5))
                        .multilineTextAlignment(.leading)
                        .lineLimit(1)
                        .frame(alignment: .leading)
                        .minimumScaleFactor(0.10)

                    Spacer()

                    Text(group.word)
                        .foregroundStyle(.midnightBlue)
                        .multilineTextAlignment(.center)
                        .frame(alignment: .leading)
                        .minimumScaleFactor(0.50)
                        .lineLimit(1)

                    Spacer()
                }
                .padding(.horizontal)
                .padding(.vertical, 12)
                .frame(width: (screenWidth * 0.33) - 20)
                .background(Color.btnPrimary)
                .cornerRadius(10)
            }
            .font(.caption)
        }
    }
}

struct StyledWordCard<Content: View>: View {
    @Binding var tabIndex: Int
    @ViewBuilder var content: Content

    var body: some View {
        TabView(selection: $tabIndex) {
            content
        }
        .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
    }
}

#Preview("12 Words") {
    NavigationStack {
        HotWalletCreateScreen(numberOfWords: .twelve)
    }
}

#Preview("24 Words") {
    NavigationStack {
        HotWalletCreateScreen(numberOfWords: .twentyFour)
    }
}
