//
//  TapSignerImportSuccess.swift
//  Cove
//
//  Created by Praveen Perera on 3/25/25.
//

import SwiftUI
import UniformTypeIdentifiers

struct TapSignerImportSuccess: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let tapSignerImport: TapSignerImportComplete

    // private
    @State private var isExportingBackup: Bool = false

    func saveWallet() {
        do {
            let manager = try WalletManager(
                tapSigner: tapSigner,
                deriveInfo: tapSignerImport.deriveInfo,
                backup: tapSignerImport.backup
            )
            app.loadAndReset(to: .selectedWallet(manager.id))
        }
        catch {
            Log.error("Failed to save wallet: \(error.localizedDescription)")
        }
    }

    var body: some View {
        VStack(spacing: 40) {
            VStack {
                HStack {
                    Button(action: { manager.popRoute() }) {
                        Image(systemName: "chevron.left")
                        Text("Back")
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
                Text("Setup Complete")
                    .font(.largeTitle)
                    .fontWeight(.bold)

                Text("Your TAPSIGNER is all setup an ready to use.")
                    .font(.subheadline)
                    .foregroundStyle(.primary.opacity(0.8))

                Text(
                    "If you haven’t already done so please download your backup and store it in a safe place. You will need this and the backup password on the back of the card to restore you wallet if you lose your TAPSIGNER."
                )
                .font(.subheadline)
                .foregroundStyle(.primary.opacity(0.8))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            }

            Spacer()

            VStack(spacing: 14) {
                Button("Continue") { saveWallet() }
                    .buttonStyle(DarkButtonStyle())
                    .padding(.horizontal)

                Button("Download Backup") { isExportingBackup = true }
                    .font(.footnote)
                    .fontWeight(.semibold)
            }
        }
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
        .fileExporter(
            isPresented: $isExportingBackup,
            document: TextDocument(text: hexEncode(bytes: tapSignerImport.backup)),
            contentType: .plainText,
            defaultFilename: "\(tapSigner.cardIdent)_backup.txt"
        ) { result in
            switch result {
            case .success:
                // TOOO: alert
                Log.debug("Successfully exported backup")
            case let .failure(error):
                // TOOO: alert
                Log.error("Failed to export backup: \(error.localizedDescription)")
            }
        }
    }
}

#Preview {
    TapSignerContainer(
        route:
        .importSuccess(
            tapSignerPreviewNew(preview: true),
            tapSignerImportCompleteNew(preview: true)
        )
    )
    .environment(AppManager.shared)
}
