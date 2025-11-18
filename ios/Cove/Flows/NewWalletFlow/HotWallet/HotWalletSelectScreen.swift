//
//  HotWalletSelectScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

enum NextScreenDialog {
    case import_
    case create
}

struct HotWalletSelectScreen: View {
    @State private var isSheetShown = false
    @State private var nextScreen: NextScreenDialog = .create

    func route(_ words: NumberOfBip39Words, importType: ImportType = .manual) -> Route {
        switch nextScreen {
        case .import_:
            HotWalletRoute.import(words, importType).intoRoute()
        case .create:
            HotWalletRoute.create(words).intoRoute()
        }
    }

    var body: some View {
        VStack(spacing: 28) {
            Spacer()

            HStack {
                DotMenuView(selected: 1, size: 5)
                Spacer()
            }

            HStack {
                Text("Do you already have a wallet?")
                    .font(.system(size: 38, weight: .semibold))
                    .lineSpacing(1.2)
                    .foregroundColor(.white)

                Spacer()
            }

            Divider()
                .overlay(.coveLightGray.opacity(0.50))

            VStack(spacing: 24) {
                Button(action: {
                    isSheetShown = true
                    nextScreen = .create
                }) {
                    Text("Create new wallet")
                        .font(.subheadline)
                        .fontWeight(.medium)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 20)
                        .padding(.horizontal, 10)
                        .background(Color.btnPrimary)
                        .foregroundColor(.midnightBlue)
                        .cornerRadius(10)
                }

                Button(action: {
                    isSheetShown = true
                    nextScreen = .import_
                }) {
                    Text("Import existing wallet")
                        .font(.subheadline)
                        .fontWeight(.medium)
                        .frame(maxWidth: .infinity)
                        .foregroundColor(.white)
                }
            }
            .confirmationDialog("Select Number of Words", isPresented: $isSheetShown) {
                if nextScreen == .import_ {
                    NavigationLink(value: route(.twentyFour, importType: .qr)) {
                        Text("Scan QR")
                    }

                    NavigationLink(value: route(.twentyFour, importType: .nfc)) {
                        Text("NFC")
                    }
                }
                NavigationLink(value: route(.twelve)) {
                    Text("12 Words")
                }
                NavigationLink(value: route(.twentyFour)) {
                    Text("24 Words")
                }
            }
        }
        .padding()
        .navigationBarTitleDisplayMode(.inline)
        .frame(maxHeight: .infinity)
        .background(
            Image(.newWalletPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: screenHeight * 0.75, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .brightness(0.05)
        )
        .background(Color.midnightBlue)
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text("Add New Wallet")
                    .font(.callout)
                    .fontWeight(.semibold)
                    .foregroundStyle(.white)
            }
        }
    }
}

#Preview {
    HotWalletSelectScreen()
}
