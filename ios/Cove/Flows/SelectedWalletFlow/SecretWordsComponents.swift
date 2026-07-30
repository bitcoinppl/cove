import SwiftUI

enum SecretWordsSensitiveAction {
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

struct SecretWordsOptionsToolbar: ToolbarContent {
    let isInDecoyMode: Bool
    let pendingAction: SecretWordsSensitiveAction?
    let isConfirmationPresented: Binding<Bool>
    let presentConfirmation: (SecretWordsSensitiveAction) -> Void
    let performSensitiveAction: (SecretWordsSensitiveAction) -> Void

    var body: some ToolbarContent {
        ToolbarItem(placement: .navigationBarTrailing) {
            Menu {
                Button {
                    presentConfirmation(.seedQr)
                } label: {
                    Label("Seed QR", systemImage: "qrcode")
                }

                // the main credential is required because the decoy PIN can never satisfy it
                if !isInDecoyMode {
                    Button {
                        presentConfirmation(.keyTeleport)
                    } label: {
                        Label("KeyTeleport", systemImage: "paperplane")
                    }
                }
            } label: {
                Image(systemName: "ellipsis.circle")
                    .foregroundStyle(.white)
            }
            .accessibilityLabel("Secret words options")
            .confirmationDialog(
                pendingAction?.confirmationTitle ?? "",
                isPresented: isConfirmationPresented,
                titleVisibility: .visible
            ) {
                if let pendingAction {
                    Button(pendingAction.confirmationButtonTitle) {
                        performSensitiveAction(pendingAction)
                    }
                }

                Button("Cancel", role: .cancel) {}
            } message: {
                if let pendingAction {
                    Text(pendingAction.confirmationMessage)
                }
            }
        }
    }
}

struct SeedQrPresentation: View {
    let words: Mnemonic?

    var body: some View {
        if let words {
            SeedQrSheetView(words: words)
        }
    }
}

struct SecretWordsCredentialVerification: View {
    let auth: AuthManager
    @Binding var succeeded: Bool

    var body: some View {
        MainCredentialVerificationView(auth: auth) {
            succeeded = true
        }
    }
}

struct SecretWordsLayout: View {
    let sizeCategory: ContentSizeCategory
    let words: Mnemonic?
    let errorMessage: String?
    let rowHeight: Double
    let numberOfRows: Int
    let numberOfColumns: Int
    private let topContentInset = 16.0

    var body: some View {
        GeometryReader { proxy in
            let compactLayout = usesCompactLayout(
                sizeCategory: sizeCategory,
                availableHeight: proxy.size.height
            )
            let contentHeight = max(proxy.size.height - topContentInset, 0)

            ScrollView {
                SecretWordsContent(
                    words: words,
                    errorMessage: errorMessage,
                    rowHeight: rowHeight,
                    numberOfRows: numberOfRows,
                    numberOfColumns: numberOfColumns,
                    usesFlexibleSpacing: !compactLayout
                )
                .frame(minHeight: contentHeight, alignment: .top)
                .safeAreaPadding(.bottom, 24)
            }
            .padding(.top, topContentInset)
            .scrollIndicators(.hidden)
        }
    }
}

struct SecretWordsContent: View {
    let words: Mnemonic?
    let errorMessage: String?
    let rowHeight: Double
    let numberOfRows: Int
    let numberOfColumns: Int
    let usesFlexibleSpacing: Bool

    var body: some View {
        VStack {
            if usesFlexibleSpacing {
                Spacer()
            }

            SecretWordsGrid(
                words: words,
                errorMessage: errorMessage,
                rowHeight: rowHeight,
                numberOfRows: numberOfRows,
                numberOfColumns: numberOfColumns
            )

            if usesFlexibleSpacing {
                Spacer()
                Spacer()
                Spacer()
            }

            SecretWordsRecoveryCopy()
        }
        .padding(.horizontal)
        .padding(.bottom)
    }
}

struct SecretWordsGrid: View {
    let words: Mnemonic?
    let errorMessage: String?
    let rowHeight: Double
    let numberOfRows: Int
    let numberOfColumns: Int

    var body: some View {
        if let words {
            GroupBox {
                ColumnMajorGrid(
                    items: words.allWords(),
                    numberOfColumns: numberOfColumns
                ) { _, word in
                    SecretWordRow(number: word.number, word: word.word)
                }
            }
            .frame(maxHeight: rowHeight * CGFloat(numberOfRows) + 32)
            .frame(width: screenWidth * 0.9)
            .font(.caption)
        } else {
            Text(errorMessage ?? "Loading...")
        }
    }
}

struct SecretWordRow: View {
    let number: UInt8
    let word: String

    var body: some View {
        HStack {
            Text("\(number).")
                .fontWeight(.medium)
                .foregroundStyle(.secondary)
                .fontDesign(.monospaced)
                .multilineTextAlignment(.leading)
                .minimumScaleFactor(0.5)

            Text(word)
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

struct SecretWordsRecoveryCopy: View {
    var body: some View {
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

struct SecretWordsPatternBackground: View {
    var body: some View {
        Image(.newWalletPattern)
            .resizable()
            .aspectRatio(contentMode: .fill)
            .frame(height: screenHeight * 0.75, alignment: .topTrailing)
            .frame(maxWidth: .infinity)
            .opacity(0.5)
    }
}

struct SeedQrSheetView: View {
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
