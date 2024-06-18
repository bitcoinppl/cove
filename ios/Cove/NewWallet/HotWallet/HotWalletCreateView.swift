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
            Text("12")
            Button(action: {
                model.dispatch(action: .updateWords(.twentyFour))
            }) {
                Text("Change Words")
            }
        }
    }
}

struct TwentyFourWordsView: View {
    var model: WalletViewModel

    var body: some View {
        VStack {
            Text("24")
            Button(action: {
                model.dispatch(action: .updateWords(.twelve))
            }) {
                Text("Change Words")
            }
        }
    }
}

#Preview {
    HotWalletCreateView(words: .twelve)
}
