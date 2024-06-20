//
//  HotWalletCreateView.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

struct HotWalletCreateView: View {
    @State private var model: WalletViewModel

    init(numberOfWords: NumberOfBip39Words) {
        self.model = WalletViewModel(numberOfWords: numberOfWords)
    }

    var body: some View {
        switch model.numberOfWords {
        case .twelve:
            TwelveWordsView(model: model, words: model.rust.bip39Words())
        case .twentyFour:
            TwentyFourWordsView(model: model, words: model.rust.bip39Words())
        }
    }
}

struct TwelveWordsView: View {
    var model: WalletViewModel
    var words: [String]

    var body: some View {
        VStack {
            HStack {
                Button("12 Words") {
                    model.dispatch(action: .updateWords(.twentyFour))
                }
            }
            .padding(.top, 50)
            .padding(.bottom, 20)

            VStack {
                Text("Please write these words down").padding(.bottom, 20)
                ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                    HStack {
                        Text("\(String(index + 1)). ")
                        Text(word)
                    }
                }
            }

            Spacer()
        }
    }
}

struct TwentyFourWordsView: View {
    var model: WalletViewModel
    var words: [String]

    var body: some View {
        VStack {
            Button("24 Words") {
                model.dispatch(action: .updateWords(.twelve))
            }
            .padding(.top, 50)
            .padding(.bottom, 20)

            VStack {
                Text("Please write these words down").padding(.bottom, 20)
                ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                    HStack {
                        Text("\(String(index + 1)). ")
                        Text(word)
                    }
                }
            }

            Spacer()
        }
    }
}

#Preview("12 Words") {
    HotWalletCreateView(numberOfWords: .twelve)
}

#Preview("24 Words") {
    HotWalletCreateView(numberOfWords: .twentyFour)
}
