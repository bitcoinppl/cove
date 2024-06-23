//
//  VerifyWordsView.swift
//  Cove
//
//  Created by Praveen Perera on 6/23/24.
//

import SwiftUI

struct VerifyWordsView: View {
    var model: WalletViewModel
    var groupedWords: [[GroupedWord]]
    @State private var tabIndex = 0

    var body: some View {
        TabView(selection: $tabIndex) {
            ForEach(groupedWords, id: \.self) { wordGroup in
                CardTab(wordGroup: wordGroup)
            }
        }
        .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
    }
}

#Preview {
    @State var model = WalletViewModel(numberOfWords: .twelve)

    return
        VerifyWordsView(model: model, groupedWords: model.rust.bip39WordsGrouped())
}

struct CardTab: View {
    var wordGroup: [GroupedWord]

    var body: some View {
        VStack(spacing: 20) {
            ForEach(Array(wordGroup.enumerated()), id: \.offset) { _, word in
                Text("\(word.number). ")
            }
        }
    }
}
