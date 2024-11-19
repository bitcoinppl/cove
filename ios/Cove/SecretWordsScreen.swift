//
//  SecretWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/22/24.
//

import SwiftUI

struct SecretWordsScreen: View {
    let id: WalletId

    // private
    @State var words: Mnemonic?
    @State var errorMessage: String?

    var cardPadding: CGFloat {
        if let words, words.allWords().count > 12 {
            8
        } else {
            50
        }
    }

    var verticalSpacing: CGFloat {
        15
    }

    var body: some View {
        Group {
            if let words {
                VStack {
                    GroupBox {
                        HStack(alignment: .top, spacing: 20) {
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
                    }
                    .padding(.horizontal, 10)
                }
            } else {
                Text(errorMessage ?? "Loading...")
            }
        }
        .onAppear {
            if words == nil {
                do {
                    words = try Mnemonic(id: id)
                } catch {
                    errorMessage = error.localizedDescription
                }
            }
        }
        .navigationTitle("Secret Words")
        .navigationBarTitleDisplayMode(.inline)
    }
}

#Preview("12") {
    SecretWordsScreen(id: WalletId(), words: Mnemonic.preview(numberOfBip39Words: .twelve))
}

#Preview("24") {
    SecretWordsScreen(id: WalletId(), words: Mnemonic.preview(numberOfBip39Words: .twentyFour))
}
