//
//  SecretWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/22/24.
//

import SwiftUI

private enum SecretWordsSensitiveAction {
    case seedQr
    case keyTeleport

    var confirmationTitle: String {
        switch self {
        case .seedQr:
            "Show Seed QR?"
        case .keyTeleport:
            "Send with KeyTeleport?"
        }
    }

    var confirmationButtonTitle: String {
        switch self {
        case .seedQr:
            "Show QR Code"
        case .keyTeleport:
            "Continue"
        }
    }

    var confirmationMessage: String {
        switch self {
        case .seedQr:
            "Your seed words are sensitive and control access to your Bitcoin. QR codes are machine-readable, so be careful who or what device you show this to."
        case .keyTeleport:
            "KeyTeleport sends this wallet's secret words to another device. Only continue if you trust the receiving device and can verify its request."
        }
    }
}

struct SecretWordsScreen: View {
    @Environment(\.sizeCategory) private var sizeCategory
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth

    let id: WalletId

    // private
    @State var words: Mnemonic?
    @State var errorMessage: String?
    @State private var pendingSensitiveAction: SecretWordsSensitiveAction?
    @State private var showSeedQrSheet = false

    let rowHeight = 30.0
    private let numberOfColumns = 2
    private let topContentInset = 16.0

    var numberOfRows: Int {
        (words?.words().count ?? 24) / numberOfColumns
    }

    private var showingSensitiveActionConfirmation: Binding<Bool> {
        Binding(
            get: { pendingSensitiveAction != nil },
            set: { isPresented in
                if !isPresented {
                    pendingSensitiveAction = nil
                }
            }
        )
    }

    private func presentConfirmation(for action: SecretWordsSensitiveAction) {
        // Wait for Menu dismissal so the action sheet can anchor to the toolbar button
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
            guard pendingSensitiveAction == nil, !showSeedQrSheet else { return }
            pendingSensitiveAction = action
        }
    }

    private func performSensitiveAction(_ action: SecretWordsSensitiveAction) {
        pendingSensitiveAction = nil

        switch action {
        case .seedQr:
            showSeedQrSheet = true
        case .keyTeleport:
            let keyTeleportManager = app.ensureKeyTeleportManager()
            keyTeleportManager.dispatch(.startSendFromWallet(id))
            app.pushRoute(RouteFactory().keyTeleportSend())
        }
    }

    var body: some View {
        GeometryReader { proxy in
            let compactLayout = usesCompactLayout(
                sizeCategory: sizeCategory,
                availableHeight: proxy.size.height
            )
            let contentHeight = max(proxy.size.height - topContentInset, 0)

            ScrollView {
                mainContent(usesFlexibleSpacing: !compactLayout)
                    .frame(minHeight: contentHeight, alignment: .top)
                    .safeAreaPadding(.bottom, 24)
            }
            .padding(.top, topContentInset)
            .scrollIndicators(.hidden)
        }
        .onAppear {
            auth.lock()
            guard words == nil else { return }
            do { words = try Mnemonic(id: id) } catch { errorMessage = error.localizedDescription }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .adaptiveToolbarStyle()
        .toolbar {
            ToolbarItem(placement: .navigationBarTrailing) {
                Menu {
                    Button {
                        presentConfirmation(for: .seedQr)
                    } label: {
                        Label("Seed QR", systemImage: "qrcode")
                    }

                    Button {
                        presentConfirmation(for: .keyTeleport)
                    } label: {
                        Label("KeyTeleport", systemImage: "paperplane")
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .foregroundStyle(.white)
                }
                .accessibilityLabel("Secret words options")
                .confirmationDialog(
                    pendingSensitiveAction?.confirmationTitle ?? "",
                    isPresented: showingSensitiveActionConfirmation,
                    titleVisibility: .visible
                ) {
                    if let pendingSensitiveAction {
                        Button(pendingSensitiveAction.confirmationButtonTitle) {
                            performSensitiveAction(pendingSensitiveAction)
                        }
                    }

                    Button("Cancel", role: .cancel) {}
                } message: {
                    if let pendingSensitiveAction {
                        Text(pendingSensitiveAction.confirmationMessage)
                    }
                }
            }
        }
        .sheet(isPresented: $showSeedQrSheet) {
            if let words {
                SeedQrSheetView(words: words)
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
    }

    private func mainContent(usesFlexibleSpacing: Bool) -> some View {
        VStack {
            if usesFlexibleSpacing {
                Spacer()
            }

            Group {
                if let words {
                    GroupBox {
                        ColumnMajorGrid(items: words.allWords(), numberOfColumns: numberOfColumns) { _, word in
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
                    .frame(maxHeight: rowHeight * CGFloat(numberOfRows) + 32)
                    .frame(width: screenWidth * 0.9)
                    .font(.caption)
                } else {
                    Text(errorMessage ?? "Loading...")
                }

                if usesFlexibleSpacing {
                    Spacer()
                    Spacer()
                    Spacer()
                }

                VStack(spacing: 12) {
                    HStack {
                        Text("Recovery Words")
                            .font(.system(size: 36, weight: .semibold))
                            .foregroundColor(.white)
                            .multilineTextAlignment(.leading)

                        Spacer()
                    }

                    HStack {
                        Text(
                            "Your secret recovery words are the only way to recover your wallet if you lose your phone or switch to a different wallet. Whoever has your recovery words, controls your Bitcoin."
                        )
                        .multilineTextAlignment(.leading)
                        .font(.footnote)
                        .foregroundStyle(.coveLightGray.opacity(0.75))
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
        .padding(.horizontal)
        .padding(.bottom)
    }
}

private struct SeedQrSheetView: View {
    let words: Mnemonic

    var body: some View {
        VStack(spacing: 16) {
            Text("Seed QR")
                .font(.title3)
                .fontWeight(.semibold)
                .padding(.top, 20)

            Text("Scan with a SeedQR-compatible device")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 40)

            if let seedQR = try? words.toSeedQrString() {
                QrCodeView(text: seedQR)
                    .padding(.horizontal, 20)
                    .padding(.top, 8)
            } else {
                Text("Failed to generate SeedQR")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .padding(.horizontal, 20)
                    .padding(.top, 8)
            }

            Spacer()
        }
        .presentationDetents([.medium, .large])
    }
}

#Preview("12") {
    SecretWordsScreen(id: WalletId(), words: Mnemonic.preview(numberOfBip39Words: .twelve))
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}

#Preview("24") {
    SecretWordsScreen(id: WalletId(), words: Mnemonic.preview(numberOfBip39Words: .twentyFour))
        .environment(AppManager.shared)
        .environment(AuthManager.shared)
}
