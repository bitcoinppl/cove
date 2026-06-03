//
//  SecretWordsScreen.swift
//  Cove
//
//  Created by Praveen Perera on 8/22/24.
//

import SwiftUI

struct SecretWordsScreen: View {
    @Environment(AppManager.self) private var app
    @Environment(AuthManager.self) private var auth

    let id: WalletId

    private enum RecoveryWordsLoadError {
        case mnemonic(MnemonicError)
        case generic(String)

        var message: String {
            switch self {
            case .mnemonic(.NotAvailable):
                "Recovery words are unavailable on this device for this wallet. This can happen after restoring to a new iPhone because Secure Enclave data does not transfer. Restore this wallet from your saved recovery words before sending more BTC to it."
            case .mnemonic(.GetWalletKeychain):
                "Cove could not access this wallet's recovery words in the keychain right now."
            case .mnemonic(.UnknownWord):
                "Cove could not decode this wallet's recovery words."
            case let .generic(message):
                message
            }
        }
    }

    // private
    @State var words: Mnemonic?
    @State private var loadError: RecoveryWordsLoadError?
    @State private var showSeedQrAlert = false
    @State private var showSeedQrSheet = false

    let rowHeight = 30.0
    private let numberOfColumns = 3

    var numberOfRows: Int {
        (words?.words().count ?? 24) / numberOfColumns
    }

    var body: some View {
        VStack {
            Spacer()

            Group {
                if let words {
                    GroupBox {
                        ColumnMajorGrid(items: words.allWords()) { _, word in
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
                } else if let loadError {
                    Text(loadError.message)
                        .multilineTextAlignment(.leading)
                        .font(.callout)
                        .foregroundStyle(.white.opacity(0.95))
                        .padding(16)
                        .frame(maxWidth: screenWidth * 0.9, alignment: .leading)
                        .background(Color.red.opacity(0.16))
                        .overlay {
                            RoundedRectangle(cornerRadius: 10)
                                .stroke(Color.red.opacity(0.30), lineWidth: 1)
                        }
                        .cornerRadius(10)
                } else {
                    ProgressView()
                        .tint(.white)
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
        .padding()
        .onAppear {
            auth.lock()
            guard words == nil else { return }
            do {
                words = try Mnemonic(id: id)
                loadError = nil
            } catch let mnemonicError as MnemonicError {
                loadError = .mnemonic(mnemonicError)
            } catch {
                loadError = .generic(error.localizedDescription)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .toolbarColorScheme(.dark, for: .navigationBar)
        .toolbar {
            if words != nil {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(action: { showSeedQrAlert = true }) {
                        Image(systemName: "qrcode")
                            .foregroundStyle(.white)
                    }
                }
            }
        }
        .alert("Show Seed QR?", isPresented: $showSeedQrAlert) {
            Button("Cancel", role: .cancel) {}
            Button("Show QR Code") { showSeedQrSheet = true }
        } message: {
            Text("Your seed words are sensitive and control access to your Bitcoin. QR codes are machine-readable, so be careful who or what device you show this to.")
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
        .tint(.white)
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
