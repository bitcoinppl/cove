//
//  TapSignerSetupSuccessView.swift
//  Cove
//
//  Created by Praveen Perera on 3/25/25.
//

import SwiftUI
import UniformTypeIdentifiers

struct TapSignerSetupSuccess: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let setup: TapSignerSetupComplete

    // private
    @State private var isExportingBackup: Bool = false
    @State private var walletId: WalletId? = nil

    func saveWallet() {
        do {
            let manager = try WalletManager(
                tapSigner: tapSigner,
                deriveInfo: setup.deriveInfo,
                backup: setup.backup
            )

            walletId = manager.id
        } catch {
            Log.error("Failed to save wallet: \(error.localizedDescription)")
        }
    }

    var body: some View {
        VStack(spacing: 40) {
            VStack {
                HStack {
                    Button(action: { app.sheetState = .none }) {
                        Text("Cancel")
                    }

                    Spacer()
                }
                .padding(.top, 20)
                .padding(.horizontal, 10)
                .foregroundStyle(.primary)
                .fontWeight(.semibold)
            }

            Spacer()

            VStack(spacing: 20) {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 100))
                    .foregroundStyle(.green)
                    .fontWeight(.light)

                VStack(spacing: 12) {
                    Text("Setup Complete")
                        .font(.largeTitle)
                        .fontWeight(.bold)

                    Text("Your TAPSIGNER ready to use.")
                        .font(.subheadline)
                        .foregroundStyle(.primary.opacity(0.8))
                }

                Text(
                    "If you haven’t already done so please download your backup and store it in a safe place. You will need this and the backup password on the back of the card to restore you wallet."
                )
                .font(.subheadline)
                .foregroundStyle(.primary.opacity(0.8))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            }

            Button(action: { isExportingBackup = true }) {
                HStack {
                    VStack(spacing: 4) {
                        HStack {
                            Text("Download Backup")
                                .font(.footnote)
                                .fontWeight(.semibold)
                                .foregroundStyle(Color.primary)
                            Spacer()
                        }

                        HStack {
                            Text("You need this backup to restore your wallet.")
                                .foregroundStyle(Color.secondary)
                            Spacer()
                        }
                    }

                    Spacer()

                    Image(systemName: "chevron.right")
                        .foregroundStyle(Color.secondary)
                }
                .padding()
                .background(Color(.systemGray6))
                .cornerRadius(10)
            }
            .font(.footnote)
            .fontWeight(.semibold)

            Spacer()

            VStack(spacing: 14) {
                Button("Continue") {
                    guard let walletId else { return saveWallet() }
                    app.selectWallet(walletId)
                    app.sheetState = .none
                }
                .buttonStyle(DarkButtonStyle())
            }
        }
        .padding(.horizontal)
        .background(
            VStack {
                Image(.chainCodePattern)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .ignoresSafeArea(edges: .all)
                    .padding(.top, 5)

                Spacer()
            }
            .opacity(0.8)
        )
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
        .onAppear {
            saveWallet()
        }
        .fileExporter(
            isPresented: $isExportingBackup,
            document: TextDocument(text: hexEncode(bytes: setup.backup)),
            contentType: .plainText,
            defaultFilename: "\(tapSigner.cardIdent)_backup.txt"
        ) { result in
            switch result {
            case .success:
                app.alertState = .init(
                    .general(
                        title: "Backup Saved!",
                        message: "Your backup has been save successfully!"
                    )
                )
            case let .failure(error):
                app.alertState = .init(
                    .general(title: "Saving Backup Failed!", message: error.localizedDescription))
            }
        }
    }
}

#Preview {
    TapSignerContainer(
        route:
            .setupSuccess(
                tapSignerPreviewNew(preview: true),
                tapSignerSetupCompleteNew(preview: true)
            )
    )
    .environment(AppManager.shared)
}
