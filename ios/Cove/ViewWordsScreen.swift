//
//  ViewWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/22/24.
//

import SwiftUI

struct ViewWordsScreen: View {
    let words: Mnemonic

    var cardPadding: CGFloat {
        if words.allWords().count > 12 {
            8
        } else {
            50
        }
    }

    var verticalSpacing: CGFloat {
        15
    }

    var body: some View {
        VStack {
            GroupBox {
                HStack(alignment: .top, spacing: 50) {
                    VStack(alignment: .leading, spacing: verticalSpacing) {
                        ForEach(words.allWords().prefix(12), id: \.number) { word in
                            Text("\(String(format: "%02d", word.number)). \(word.word)")
                                .fontDesign(.monospaced)
                                .multilineTextAlignment(.leading)
                        }
                    }

                    if words.allWords().count > 12 {
                        VStack(alignment: .leading, spacing: verticalSpacing) {
                            ForEach(words.allWords().dropFirst(12), id: \.number) { word in
                                Text("\(String(format: "%02d", word.number)). \(word.word)")
                                    .fontDesign(.monospaced)
                                    .multilineTextAlignment(.leading)
                            }
                        }
                    }
                }
                .padding(.horizontal, cardPadding)
                .padding(.vertical, 30)
            }
            .padding(.horizontal, 10)
        }
        .padding(.vertical, 30)
        .navigationTitle("Secret Words")
    }
}

#Preview("12") {
    ViewWordsScreen(words: Mnemonic.preview(numberOfBip39Words: .twelve))
}

#Preview("24") {
    ViewWordsScreen(words: Mnemonic.preview(numberOfBip39Words: .twentyFour))
}
