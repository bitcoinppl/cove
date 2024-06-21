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

    var body: some View {
        ZStack {
            RadialGradient(
                gradient: Gradient(colors: [
                    Color.red.opacity(0.9),
                    Color.orange.opacity(0.6),
                ]),
                center: .center, startRadius: 2, endRadius: 650
            )
            .edgesIgnoringSafeArea(.all)

            VStack(spacing: 20) {
                StyledButton("Switch to 24 Word") {
                    model.dispatch(action: .updateWords(.twentyFour))
                }.padding(.top, 20)

                Text("Please write these words down")
                    .font(.title3)
                    .foregroundColor(.white)

                StyledWordCard {
                    ForEach(0..<2) { pageIndex in
                        WordCardView(
                            words: Array(model.bip39Words[pageIndex * 6..<min((pageIndex + 1) * 6, model.bip39Words.count)]),
                            startIndex: pageIndex * 6
                        )
                    }
                }

                Spacer()
            }
        }
    }
}

struct TwentyFourWordsView: View {
    var model: WalletViewModel

    var body: some View {
        OrangeGradientBackgroundView {
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
                        .fontWeight(.medium)
                }
            }
        }
        .padding()
        .foregroundColor(.white)
    }
}

#Preview("12 Words") {
    HotWalletCreateView(numberOfWords: .twelve)
}

#Preview("24 Words") {
    HotWalletCreateView(numberOfWords: .twentyFour)
}

struct StyledWordCard<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        TabView {
            content
        }
        .tabViewStyle(PageTabViewStyle(indexDisplayMode: .automatic))
        .frame(height: 300)
        .background(.ultraThinMaterial)
        .cornerRadius(20)
        .overlay(
            RoundedRectangle(cornerRadius: 20)
                .stroke(Color.white.opacity(0.2), lineWidth: 1)
        )
        .shadow(color: Color.black.opacity(0.1), radius: 10, x: 0, y: 10)
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
