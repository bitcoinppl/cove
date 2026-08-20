//
//  TapSignerSetupSuccessView.swift
//  Cove
//
//  Created by Praveen Perera on 3/25/25.
//

import CoveCore
import SwiftUI

struct TapSignerSetupSuccess: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let setup: TapSignerSetupComplete

    /// private
    @State private var walletId: WalletId? = nil

    func saveWallet() {
        do {
            let walletManager = try WalletManager(
                tapSigner: tapSigner,
                deriveInfo: setup.deriveInfo,
                backup: setup.backup,
                birthday: setup.birthday
            )

            walletId = walletManager.id
        } catch {
            Log.error("Failed to save TapSigner wallet")
        }
    }

    var body: some View {
        TapSignerAdaptiveLayout(
            compactContentBottomPadding: 40,
            compactSafeAreaBottomPadding: 0
        ) { usesFlexibleSpacing in
            TapSignerSetupSuccessContent(
                usesFlexibleSpacing: usesFlexibleSpacing,
                cancelAction: cancel,
                exportBackupAction: exportBackup,
                continueAction: continueToWallet
            )
        }
        .background(TapSignerResultBackground())
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
        .onAppear(perform: saveWallet)
    }

    private func cancel() {
        manager.cancel()
        app.sheetState = .none
    }

    private func exportBackup() {
        let content = hexEncode(bytes: setup.backup)
        let filename = "\(tapSigner.identFileNamePrefix())_backup.txt"

        ShareSheet.present(data: content, filename: filename) { success in
            if !success {
                Log.warn("Backup export cancelled or failed")
            }
        }
    }

    private func continueToWallet() {
        guard let walletId else {
            saveWallet()
            return
        }

        manager.cancel()
        app.selectWallet(walletId)
        app.sheetState = .none
    }
}

private struct TapSignerSetupSuccessContent: View {
    let usesFlexibleSpacing: Bool
    let cancelAction: () -> Void
    let exportBackupAction: () -> Void
    let continueAction: () -> Void

    private var contentSpacing: CGFloat {
        usesFlexibleSpacing ? 40 : 24
    }

    var body: some View {
        VStack(spacing: contentSpacing) {
            TapSignerTopActionHeader("Cancel", action: cancelAction)

            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

            TapSignerSetupSuccessDescription()
            TapSignerBackupButton(action: exportBackupAction)
            TapSignerFlexibleSpacer(enabled: usesFlexibleSpacing)

            VStack(spacing: 14) {
                Button("Continue", action: continueAction)
                    .buttonStyle(DarkButtonStyle())
            }
        }
        .padding(.horizontal)
    }
}

private struct TapSignerSetupSuccessDescription: View {
    var body: some View {
        VStack(spacing: 20) {
            TapSignerResultStatus(
                systemImage: "checkmark.circle.fill",
                color: .green,
                title: "Setup Complete",
                message: "Your TAPSIGNER ready to use."
            )

            Text(
                "If you haven’t already done so please download your backup and store it in a safe place. You will need this and the backup password on the back of the card to restore you wallet."
            )
            .font(.subheadline)
            .foregroundStyle(.primary.opacity(0.8))
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
        }
    }
}

private struct TapSignerBackupButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
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
