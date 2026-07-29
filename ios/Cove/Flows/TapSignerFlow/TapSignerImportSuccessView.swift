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

    let tapSigner: TapSigner
    let deriveInfo: DeriveInfo

    /// private
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
        TapSignerAdaptiveLayout { usesFlexibleSpacing in
            TapSignerSuccessContent(
                usesFlexibleSpacing: usesFlexibleSpacing,
                title: "Import Complete",
                message: "Your TAPSIGNER ready to use.",
                cancelAction: cancel,
                continueAction: continueToWallet
            )
        }
        .onAppear(perform: saveWallet)
        .background(TapSignerResultBackground())
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }

    private func cancel() {
        app.sheetState = .none
    }

    private func continueToWallet() {
        guard let walletId else {
            saveWallet()
            return
        }

        app.selectWallet(walletId)
        app.sheetState = .none
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
