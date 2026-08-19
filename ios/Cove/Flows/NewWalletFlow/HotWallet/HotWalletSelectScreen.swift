//
//  HotWalletSelectScreen.swift
//  Cove
//
//  Created by Praveen Perera on 6/17/24.
//

import SwiftUI

enum NextScreenDialog: Equatable {
    case import_
    case create
}

struct HotWalletSelectScreen: View {
    @Environment(\.sizeCategory) private var sizeCategory

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
        HotWalletSelectionLayout(
            sizeCategory: sizeCategory,
            isSheetShown: $isSheetShown,
            nextScreen: $nextScreen,
            route: route
        )
        .background(
            Image(.newWalletPattern)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(height: screenHeight * 0.75, alignment: .topTrailing)
                .frame(maxWidth: .infinity)
                .opacity(0.5)
        )
        .background(Color.midnightBlue)
        .navigationTitle("Add New Wallet")
        .navigationBarTitleDisplayMode(.inline)
        .toolbarColorScheme(.dark, for: .navigationBar)
    }
}

private struct HotWalletSelectionLayout: View {
    let sizeCategory: ContentSizeCategory
    @Binding var isSheetShown: Bool
    @Binding var nextScreen: NextScreenDialog
    let route: (NumberOfBip39Words, ImportType) -> Route

    var body: some View {
        GeometryReader { proxy in
            let scrollableLayout = usesCompactLayout(
                sizeCategory: sizeCategory,
                availableHeight: proxy.size.height
            )

            Group {
                if scrollableLayout {
                    ScrollView {
                        HotWalletSelectionBottomLayout(
                            isSheetShown: $isSheetShown,
                            nextScreen: $nextScreen,
                            route: route
                        )
                        .frame(minHeight: proxy.size.height)
                    }
                    .scrollIndicators(.hidden)
                } else {
                    HotWalletSelectionBottomLayout(
                        isSheetShown: $isSheetShown,
                        nextScreen: $nextScreen,
                        route: route
                    )
                    .frame(width: proxy.size.width, height: proxy.size.height)
                }
            }
        }
    }
}

private struct HotWalletSelectionBottomLayout: View {
    @Binding var isSheetShown: Bool
    @Binding var nextScreen: NextScreenDialog
    let route: (NumberOfBip39Words, ImportType) -> Route

    var body: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)

            HotWalletSelectionPrompt()
                .padding(.horizontal)
                .padding(.bottom, 28)

            VStack(spacing: 28) {
                Divider()
                    .overlay(.coveLightGray.opacity(0.50))

                HotWalletChoiceActions(
                    isSheetShown: $isSheetShown,
                    nextScreen: $nextScreen,
                    route: route
                )
            }
            .padding(.horizontal)
            .padding(.top, 16)
            .padding(.bottom, 56)
        }
    }
}

private struct HotWalletSelectionPrompt: View {
    var body: some View {
        VStack(spacing: 28) {
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
        }
    }
}

private struct HotWalletChoiceActions: View {
    @Binding var isSheetShown: Bool
    @Binding var nextScreen: NextScreenDialog
    let route: (NumberOfBip39Words, ImportType) -> Route

    var body: some View {
        VStack(spacing: 24) {
            HotWalletChoiceButtons(
                isSheetShown: $isSheetShown,
                nextScreen: $nextScreen,
                route: route
            )
        }
    }
}

private struct HotWalletChoiceButtons: View {
    @Binding var isSheetShown: Bool
    @Binding var nextScreen: NextScreenDialog
    let route: (NumberOfBip39Words, ImportType) -> Route

    var body: some View {
        VStack(spacing: 24) {
            Button(action: selectCreateWallet) {
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
            .confirmationDialog(
                "Select Number of Words",
                isPresented: isPresenting(.create)
            ) {
                HotWalletWordCountOptions(
                    includesImportOptions: false,
                    qrRoute: route(.twentyFour, .qr),
                    nfcRoute: route(.twentyFour, .nfc),
                    twelveWordRoute: route(.twelve, .manual),
                    twentyFourWordRoute: route(.twentyFour, .manual)
                )
            }

            Button(action: selectImportWallet) {
                Text("Import existing wallet")
                    .font(.subheadline)
                    .fontWeight(.medium)
                    .frame(maxWidth: .infinity)
                    .foregroundColor(.white)
            }
            .confirmationDialog(
                "Select Number of Words",
                isPresented: isPresenting(.import_)
            ) {
                HotWalletWordCountOptions(
                    includesImportOptions: true,
                    qrRoute: route(.twentyFour, .qr),
                    nfcRoute: route(.twentyFour, .nfc),
                    twelveWordRoute: route(.twelve, .manual),
                    twentyFourWordRoute: route(.twentyFour, .manual)
                )
            }
        }
    }

    private func isPresenting(_ choice: NextScreenDialog) -> Binding<Bool> {
        Binding(
            get: { isSheetShown && nextScreen == choice },
            set: { isPresented in
                if !isPresented, nextScreen == choice { isSheetShown = false }
            }
        )
    }

    private func selectCreateWallet() {
        nextScreen = .create
        isSheetShown = true
    }

    private func selectImportWallet() {
        nextScreen = .import_
        isSheetShown = true
    }
}

private struct HotWalletWordCountOptions: View {
    let includesImportOptions: Bool
    let qrRoute: Route
    let nfcRoute: Route
    let twelveWordRoute: Route
    let twentyFourWordRoute: Route

    var body: some View {
        if includesImportOptions {
            NavigationLink(value: qrRoute) {
                Text("Scan QR")
            }

            NavigationLink(value: nfcRoute) {
                Text("NFC")
            }
        }

        NavigationLink(value: twelveWordRoute) {
            Text("12 Words")
        }

        NavigationLink(value: twentyFourWordRoute) {
            Text("24 Words")
        }
    }
}

#Preview {
    HotWalletSelectScreen()
}
