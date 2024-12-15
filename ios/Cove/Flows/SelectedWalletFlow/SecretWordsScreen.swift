//
//  SecretWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/22/24.
//

import SwiftUI

struct SecretWordsScreen: View {
    @Environment(AppManager.self) private var app
    
    let id: WalletId

    // private
    @State var words: Mnemonic?
    @State var errorMessage: String?

    var verticalSpacing: CGFloat {
        15
    }

    let rowHeight = 30.0
    var numberOfRows: Int {
        (words?.words().count ?? 24) / 3
    }

    var rows: [GridItem] {
        Array(repeating: .init(.fixed(rowHeight)), count: numberOfRows)
    }
    
    var body: some View {
        VStack {
            Spacer()
                
            Group {
                if let words {
                    GroupBox {
                        LazyHGrid(rows: rows, spacing: 12) {
                            ForEach(words.allWords(), id: \.number) { word in
                                HStack {
                                    Text("\(word.number).")
                                        .fontWeight(.medium)
                                        .foregroundStyle(.secondary)
                                        .fontDesign(.monospaced)
                                        .multilineTextAlignment(.leading)
                                        .minimumScaleFactor(0.5)
                                        
                                    Text(word.word)
                                        .fontWeight(.bold)
                                        .fontDesign(.monospaced)
                                        .multilineTextAlignment(.leading)
                                        .minimumScaleFactor(0.75)
                                        .lineLimit(1)
                                        .fixedSize()
                                        
                                    Spacer()
                                }
                            }
                        }
                    }
                    .frame(maxHeight: rowHeight * CGFloat(numberOfRows) + 32)
                    .frame(width: screenWidth * 0.9)
                    .font(.caption)
                } else {
                    Text(errorMessage ?? "Loading...")
                }
                    
                Spacer()
                Spacer()
                Spacer()
                    
                VStack(spacing: 12) {
                    HStack {
                        Text("Recovery Words")
                            .font(.system(size: 36, weight: .semibold))
                            .foregroundColor(.white)
                            .multilineTextAlignment(.leading)
                            
                        Spacer()
                    }
                        
                    HStack {
                        Text("Your secret recovery words are the only way to recover your wallet if you lose your phone or switch to a different wallet. Whoever has you recovery words, controls your Bitcoin.")
                            .multilineTextAlignment(.leading)
                            .font(.footnote)
                            .foregroundStyle(.lightGray.opacity(0.75))
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
                }
            }
        }
        .padding()
        .onAppear {
            app.lock()
            guard words == nil else { return }
            do { words = try Mnemonic(id: id) }
            catch { errorMessage = error.localizedDescription }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text("Recovery Words")
                    .foregroundStyle(.white)
                    .font(.callout)
                    .fontWeight(.semibold)
            }
        }
        .background(
            Image(.newWalletPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: screenHeight * 0.75, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .opacity(0.5)
        )
        .background(Color.midnightBlue)
        .tint(.white)
    }
}

#Preview("12") {
    SecretWordsScreen(id: WalletId(), words: Mnemonic.preview(numberOfBip39Words: .twelve))
        .environment(AppManager())
}

#Preview("24") {
    SecretWordsScreen(id: WalletId(), words: Mnemonic.preview(numberOfBip39Words: .twentyFour))
        .environment(AppManager())
}
