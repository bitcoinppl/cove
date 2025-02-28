//
//  HotWalletCreateScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

struct HotWalletCreateScreen: View {
    @State private var manager: PendingWalletManager

    init(numberOfWords: NumberOfBip39Words) {
        manager = PendingWalletManager(numberOfWords: numberOfWords)
    }

    var body: some View {
        WordsView(manager: manager, groupedWords: manager.rust.bip39WordsGrouped())
    }
}

private let columns = [
    GridItem(.flexible()),
    GridItem(.flexible()),
    GridItem(.flexible()),
]

struct WordsView: View {
    var manager: PendingWalletManager
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

            HStack {
                Text(
                    "Your secret recovery words are the only way to recover your wallet if you lose your phone or switch to a different wallet. Whoever has your recovery words, controls your Bitcoin."
                )
                .font(.subheadline)
                .foregroundStyle(.coveLightGray)
                .multilineTextAlignment(.leading)
                .opacity(0.70)
                .fixedSize(horizontal: false, vertical: true)

                Spacer()
            }

            HStack {
                Text("Please save these words in a secure location.")
                    .font(.subheadline)
                    .multilineTextAlignment(.leading)
                    .fontWeight(.bold)
                    .foregroundStyle(.white)
                    .opacity(0.9)

                Spacer()
            }

            Divider()
                .overlay(.coveLightGray.opacity(0.50))

            VStack(spacing: 24) {
                Group {
                    if tabIndex == lastIndex {
                        Button(action: {
                            do {
                                // save the wallet
                                let walletId = try manager.rust.saveWallet().id

                                navigate(
                                    HotWalletRoute.verifyWords(walletId).intoRoute()
                                )
                            } catch {
                                // TODO: handle, maybe show an alert?
                                Log.error("Error \(error)")
                            }
                        }) {
                            Text("Save Wallet")
                                .font(.subheadline)
                                .fontWeight(.medium)
                                .frame(maxWidth: .infinity)
                                .contentShape(Rectangle())
                                .padding(.vertical, 20)
                                .padding(.horizontal, 10)
                                .background(Color.btnPrimary)
                                .foregroundColor(.midnightBlue)
                                .cornerRadius(10)
                        }
                    } else {
                        Button(action: {
                            withAnimation { tabIndex += 1 }
                        }) {
                            Text("Next")
                                .font(.subheadline)
                                .fontWeight(.medium)
                                .frame(maxWidth: .infinity)
                                .contentShape(Rectangle())
                                .padding(.vertical, 20)
                                .padding(.horizontal, 10)
                                .background(Color.btnPrimary)
                                .foregroundColor(.midnightBlue)
                                .cornerRadius(10)
                        }
                    }
                }
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
        .navigationBarBackButtonHidden(true)
    }
}

struct WordCardView: View {
    let words: [GroupedWord]

    var body: some View {
        LazyVGrid(columns: columns, spacing: 20) {
            ForEach(words, id: \.self) { group in
                HStack(spacing: 0) {
                    Text("\(String(format: "%d", group.number)). ")
                        .fontWeight(.medium)
                        .foregroundColor(.black.opacity(0.5))
                        .multilineTextAlignment(.leading)
                        .lineLimit(1)
                        .frame(alignment: .leading)
                        .minimumScaleFactor(0.10)

                    Spacer()

                    Text(group.word)
                        .fontWeight(.medium)
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
            content.padding(.bottom, 20)
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
