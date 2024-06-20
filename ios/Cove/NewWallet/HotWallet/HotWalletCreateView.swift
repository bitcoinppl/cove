//
//  HotWalletCreateView.swift
//  Cove
//
//  Created by Praveen Perera on 6/18/24.
//

import SwiftUI

struct HotWalletCreateView: View {
    @State private var model: WalletViewModel

    init(words: NumberOfBip39Words) {
        self.model = WalletViewModel(words: words)
    }

    var body: some View {
        switch model.words {
        case .twelve:
            TwelveWordsView(model: model)
        case .twentyFour:
            TwentyFourWordsView(model: model)
        }
    }
}

struct TwelveWordsView: View {
    var model: WalletViewModel

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
                Text("Please write these words down").padding(.bottom, 30)
                Text(model.rust.bip39Words())
            }

            Spacer()
        }
    }
}

struct TwentyFourWordsView: View {
    var model: WalletViewModel

    var body: some View {
        VStack {
            Button("24 Words") {
                model.dispatch(action: .updateWords(.twelve))
            }
            .padding(.top, 50)
            .padding(.bottom, 20)

            VStack {
                Text("Please write these words down").padding(.bottom, 30)
                Text(model.rust.bip39Words())
            }

            Spacer()
        }
    }
}

#Preview("12 Words") {
    HotWalletCreateView(words: .twelve)
}

#Preview("24 Words") {
    HotWalletCreateView(words: .twentyFour)
}
