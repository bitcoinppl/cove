//
//  HotWalletSelectView.swift
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
        VStack(spacing: 20) {
            Text("Select Wallet Option")
                .font(.largeTitle)
                .fontWeight(.bold)
                .padding(.top)

            Spacer()

            Button(action: { isSheetShown = true; nextScreen = .create }) {
                HStack {
                    Image(systemName: "plus.circle.fill")
                    Text("Create Wallet")
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 25)
                .background(Color.blue)
                .foregroundColor(.white)
                .cornerRadius(10)
            }
            Button(action: { isSheetShown = true; nextScreen = .import_ }) {
                HStack {
                    Image(systemName: "arrow.down.circle.fill")
                    Text("Import Wallet")
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 25)
                .background(Color.secondary.opacity(0.1))
                .foregroundColor(.primary)
                .cornerRadius(10)
            }

            Spacer()
            Spacer()
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
        .padding()
        .navigationBarTitleDisplayMode(.inline)
    }
}

#Preview {
    HotWalletSelectScreen()
}
