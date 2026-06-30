//
//  HotWalletImportScreenSections.swift
//  Cove
//
//  Created by Praveen Perera on 6/30/26.
//

import SwiftUI

struct HotWalletImportButton: View {
    let importWallet: () -> Void

    var body: some View {
        Button("Import wallet", action: importWallet)
            .accessibilityIdentifier("hotWalletImport.import")
            .font(.subheadline)
            .fontWeight(.medium)
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
            .padding(.vertical, 20)
            .background(Color.btnPrimary)
            .foregroundColor(.midnightBlue)
            .cornerRadius(10)
    }
}

struct HotWalletImportKeyboardToolbar: View {
    let filteredSuggestions: [String]
    let accessoryHeight: CGFloat
    let selectWord: (String) -> Void

    var body: some View {
        HStack {
            ForEach(filteredSuggestions, id: \.self) { word in
                Spacer()

                Button(word) { selectWord(word) }
                    .foregroundColor(.primary)

                Spacer()

                if filteredSuggestions.count > 1, filteredSuggestions.last != word {
                    Divider()
                }
            }
        }
        .frame(maxWidth: .infinity)
        .frame(height: accessoryHeight)
        .background(.regularMaterial)
        .modifier(KeyboardToolbarShapeModifier())
    }
}

struct HotWalletImportMainContent: View {
    let keyboardIsShowing: Bool
    let isCompactLayout: Bool
    let numberOfWords: NumberOfBip39Words
    @Binding var tabIndex: Int
    @Binding var enteredWords: [[String]]
    @Binding var filteredSuggestions: [String]
    @FocusState.Binding var focusField: ImportFieldNumber?
    let onPasteMnemonic: (String) -> Void
    let importWallet: () -> Void

    var body: some View {
        VStack {
            if !keyboardIsShowing {
                Spacer()
            }

            if isCompactLayout {
                ScrollView {
                    card
                        .frame(idealHeight: 300)
                }
                .scrollIndicators(.hidden)
            } else {
                card
            }

            if numberOfWords == .twentyFour {
                DotMenuView(selected: tabIndex, size: 5, total: 2)
            }

            Spacer()

            HotWalletImportButton(importWallet: importWallet)
        }
    }

    private var card: some View {
        HotWalletImportCard(
            numberOfWords: numberOfWords,
            onPasteMnemonic: onPasteMnemonic,
            tabIndex: $tabIndex,
            enteredWords: $enteredWords,
            filteredSuggestions: $filteredSuggestions,
            focusField: $focusField
        )
    }
}
