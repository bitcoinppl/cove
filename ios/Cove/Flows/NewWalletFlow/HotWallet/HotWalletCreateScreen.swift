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
        WordsView(manager: manager)
    }
}

struct WordsView: View {
    @Environment(\.sizeCategory) var sizeCategory

    var manager: PendingWalletManager

    // private
    @State private var groupedWords: [[GroupedWord]]
    @State private var tabIndex = 0
    @State private var showConfirmationAlert = false
    @Environment(\.dismiss) private var dismiss
    @Environment(\.navigate) private var navigate
    @Environment(AppManager.self) private var app

    init(manager: PendingWalletManager, initialTabIndex: Int = 0) {
        self.manager = manager
        self.groupedWords = manager.rust.bip39WordsGrouped()
        self.tabIndex = initialTabIndex
    }

    var lastIndex: Int {
        groupedWords.count - 1
    }

    var body: some View {
        GeometryReader { proxy in
            let scrollableLayout = usesScrollableLayout(availableHeight: proxy.size.height)

            Group {
                if scrollableLayout {
                    VStack(spacing: 0) {
                        ScrollView {
                            recoveryWordsContent(
                                compactHeight: scrollableLayout,
                                includesPrimaryAction: false
                            )
                            .padding(.bottom, 24)
                        }
                        .scrollIndicators(.hidden)

                        compactBottomAction
                    }
                    .frame(width: proxy.size.width, height: proxy.size.height)
                    .background(
                        Color.midnightBlue
                            .ignoresSafeArea(.all)
                    )

                } else {
                    recoveryWordsContent(
                        compactHeight: scrollableLayout,
                        includesPrimaryAction: true
                    )
                }
            }
        }
    }

    private func usesScrollableLayout(availableHeight: CGFloat) -> Bool {
        sizeCategory >= .extraExtraLarge || availableHeight <= 812
    }

    private func recoveryWordsContent(compactHeight: Bool, includesPrimaryAction: Bool) -> some View {
        RecoveryWordsContent(
            groupedWords: groupedWords,
            tabIndex: $tabIndex,
            lastIndex: lastIndex,
            showConfirmationAlert: $showConfirmationAlert,
            compactHeight: compactHeight,
            includesPrimaryAction: includesPrimaryAction,
            saveWallet: saveWallet,
            dismiss: { dismiss() }
        )
    }

    private var compactBottomAction: some View {
        VStack(spacing: 16) {
            Divider()
                .overlay(.coveLightGray.opacity(0.50))

            RecoveryWordsPrimaryActionButton(
                tabIndex: $tabIndex,
                lastIndex: lastIndex,
                saveWallet: saveWallet
            )
        }
        .padding(.horizontal)
        .padding(.top, 12)
        .padding(.bottom, 56)
        .background(Color.midnightBlue)
    }

    private func saveWallet() {
        do {
            let result = try manager.rust.saveWallet()
            app.resetRoute(to: result.routes)
        } catch {
            Log.error("Error \(error)")
        }
    }
}

struct RecoveryWordsContent: View {
    let groupedWords: [[GroupedWord]]
    @Binding var tabIndex: Int
    let lastIndex: Int
    @Binding var showConfirmationAlert: Bool
    let compactHeight: Bool
    let includesPrimaryAction: Bool
    let saveWallet: () -> Void
    let dismiss: () -> Void

    var body: some View {
        VStack(spacing: 24) {
            StyledWordCard(tabIndex: $tabIndex, compactHeight: compactHeight) {
                ForEach(Array(groupedWords.enumerated()), id: \.offset) { index, wordGroup in
                    WordCardView(words: wordGroup).tag(index)
                }
            }

            if !compactHeight {
                Spacer()
            }

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

            if includesPrimaryAction {
                Divider()
                    .overlay(.coveLightGray.opacity(0.50))

                VStack(spacing: 24) {
                    primaryActionButton
                }
            }
        }
        .padding()
        .navigationBarTitleDisplayMode(.inline)
        .adaptiveToolbarStyle()
        .toolbarColorScheme(.dark, for: .navigationBar)
        .toolbarBackground(Color.midnightBlue, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
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
        .navigationTitle("Backup your wallet")
        .navigationBarTitleDisplayMode(.inline)
        .toolbarColorScheme(.dark, for: .navigationBar)
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

    private var primaryActionButton: some View {
        RecoveryWordsPrimaryActionButton(
            tabIndex: $tabIndex,
            lastIndex: lastIndex,
            saveWallet: saveWallet
        )
    }
}

struct RecoveryWordsPrimaryActionButton: View {
    @Binding var tabIndex: Int
    let lastIndex: Int
    let saveWallet: () -> Void

    var body: some View {
        if tabIndex == lastIndex {
            Button(action: saveWallet) {
                primaryActionLabel("Save Wallet")
            }
        } else {
            Button(action: {
                withAnimation { tabIndex += 1 }
            }) {
                primaryActionLabel("Next")
            }
        }
    }

    private func primaryActionLabel(_ title: String) -> some View {
        Text(title)
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

struct WordCardView: View {
    @Environment(\.sizeCategory) var sizeCategory
    let words: [GroupedWord]

    private let columnCount = 3
    private let columnSpacing: CGFloat = 12

    var body: some View {
        GeometryReader { proxy in
            ColumnMajorGrid(items: words, numberOfColumns: columnCount, spacing: columnSpacing) { _, group in
                wordCard(group, width: wordCardWidth(availableWidth: proxy.size.width))
            }
        }
    }

    private func wordCard(_ group: GroupedWord, width: CGFloat) -> some View {
        HStack(spacing: 0) {
            Text("\(String(format: "%d", group.number)). ")
                .fontWeight(.medium)
                .foregroundColor(.black.opacity(0.5))
                .multilineTextAlignment(.leading)
                .frame(alignment: .leading)
                .minimumScaleFactor(0.8)
                .lineLimit(sizeCategory >= .extraExtraLarge ? 3 : 1)
                .font(isMiniDeviceOrLargeText(sizeCategory) ? .caption2 : .caption)

            Spacer(minLength: 4)

            Text(group.word)
                .fontWeight(.medium)
                .foregroundStyle(.midnightBlue)
                .multilineTextAlignment(.center)
                .frame(alignment: .leading)
                .minimumScaleFactor(0.2)
                .lineLimit(sizeCategory >= .extraExtraLarge ? 5 : 1)
                .font(isMiniDeviceOrLargeText(sizeCategory) ? .caption2 : .footnote)

            Spacer(minLength: 4)
        }
        .padding(.horizontal, isMiniDeviceOrLargeText(sizeCategory) ? 8 : 10)
        .padding(.vertical, 12)
        .frame(width: width)
        .background(Color.btnPrimary)
        .cornerRadius(10)
        .contextMenu {
            isMiniDeviceOrLargeText(sizeCategory)
                ? Button(action: {}) {
                    Text("\(String(format: "%d", group.number)). \(group.word)")
                } : nil
        }
    }

    private func wordCardWidth(availableWidth: CGFloat) -> CGFloat {
        let totalColumnSpacing = columnSpacing * CGFloat(columnCount - 1)

        return max((availableWidth - totalColumnSpacing) / CGFloat(columnCount), 0)
    }
}

struct StyledWordCard<Content: View>: View {
    @Environment(\.sizeCategory) var sizeCategory

    @Binding var tabIndex: Int
    let compactHeight: Bool
    @ViewBuilder var content: Content

    var body: some View {
        let tabView = TabView(selection: $tabIndex) {
            content.padding(.bottom, 40)
        }
        .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))

        if compactHeight {
            tabView.frame(height: isMiniDeviceOrLargeText(sizeCategory) ? 320 : 260)
        } else {
            tabView
        }
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
