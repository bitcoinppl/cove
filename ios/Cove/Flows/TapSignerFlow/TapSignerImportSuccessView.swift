//
//  TapSignerImportSuccessView.swift
//  Cove
//
//  Created by Praveen Perera on 3/27/25.
//

import SwiftUI
import UniformTypeIdentifiers

struct TapSignerImportSuccess: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let deriveInfo: DeriveInfo

    // private
    @State private var walletId: WalletId? = nil

    func saveWallet() {
        do {
            let manager = try WalletManager(tapSigner: tapSigner, deriveInfo: deriveInfo)
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
                    Text("Import Complete")
                        .font(.largeTitle)
                        .fontWeight(.bold)

                    Text("Your TAPSIGNER ready to use.")
                        .font(.subheadline)
                        .foregroundStyle(.primary.opacity(0.8))
                }
            }

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
        .onAppear {
            saveWallet()
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
    }
}

#Preview {
    TapSignerContainer(
        route:
        .importSuccess(
            tapSignerPreviewNew(preview: true),
            tapSignerSetupCompleteNew(preview: true).deriveInfo
        )
    )
    .environment(AppManager.shared)
}
