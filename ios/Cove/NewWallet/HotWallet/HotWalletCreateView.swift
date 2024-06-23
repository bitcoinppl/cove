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
            TwelveWordsView(model: model)
        case .twentyFour:
            TwentyFourWordsView(model: model)
        }
    }
}

struct TwelveWordsView: View {
    var model: WalletViewModel
    @State private var tabIndex = 0

    var body: some View {
        SunsetWave {
            VStack {
                Spacer()

                Text("Please write these words down")
                    .font(.title2)
                    .fontWeight(.semibold)
                    .foregroundColor(.white.opacity(0.75))

                StyledWordCard(tabIndex: $tabIndex) {
                    ForEach(0..<2) { pageIndex in
                        WordCardView(
                            words: Array(model.bip39Words[pageIndex * 6..<min((pageIndex + 1) * 6, model.bip39Words.count)]),
                            startIndex: pageIndex * 6
                        )
                    }
                }.padding()

                Spacer()

                if tabIndex == 0 {
                    Button("Next") {
                        model.dispatch(action: .updateWords(.twentyFour))
                    }
                    .background(.white)
                    .padding(.top, 50)
                } else {
                    StyledButton("Switch to 24 Word") {
                        model.dispatch(action: .updateWords(.twentyFour))
                    }
                    .padding(.top, 50)
                }

                Spacer()
            }
        }
    }
}

struct TwentyFourWordsView: View {
    var model: WalletViewModel

    var body: some View {
        SunsetWave {
            VStack {
                Button("24 Words") {
                    model.dispatch(action: .updateWords(.twelve))
                }
                .padding(.top, 50)
                .padding(.bottom, 20)

                VStack {
                    Text("Please write these words down").padding(.bottom, 20)
                    ForEach(Array(model.bip39Words.enumerated()), id: \.offset) { index, word in
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
}

struct WordCardView: View {
    let words: [String]
    let startIndex: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                HStack {
                    Text("\(startIndex + index + 1).")
                        .foregroundColor(.secondary)

                    Text(word)
                        .font(.headline)
                }
            }
        }
        .padding()
        .foregroundColor(.white)
    }
}

struct StyledWordCard<Content: View>: View {
    @Binding var tabIndex: Int
    @ViewBuilder var content: Content

    var body: some View {
        GlassCard {
            TabView(selection: $tabIndex) {
                content
            }
            .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
        }
        .frame(height: 300)
        .padding()
    }
}

struct StyledButton: View {
    let text: String
    let action: () -> Void

    init(_ text: String, action: @escaping () -> Void) {
        self.text = text
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Text(text)
                .padding()
                .background(
                    LinearGradient(
                        gradient: Gradient(colors: [
                            Color(red: 0.2, green: 0.4, blue: 1.0),
                            Color(red: 0.1, green: 0.5, blue: 1.0),
                        ]),
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
                .foregroundColor(.white)
                .cornerRadius(10)
                .shadow(color: Color.black.opacity(0.3), radius: 5, x: 0, y: 2)
        }
    }
}

#Preview("12 Words") {
    HotWalletCreateView(numberOfWords: .twelve)
}

#Preview("24 Words") {
    HotWalletCreateView(numberOfWords: .twentyFour)
}
