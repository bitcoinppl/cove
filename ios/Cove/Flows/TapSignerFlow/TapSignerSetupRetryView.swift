//
//  TapSignerSetupRetryView.swift
//  Cove
//
//  Created by Praveen Perera on 3/25/25.
//

import SwiftUI
import UniformTypeIdentifiers

struct TapSignerSetupRetry: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let response: SetupCmdResponse

    @State private var isRunning = false
    @State private var didSaveBackup = false

    private var availableBackup: Data? {
        switch response {
        case let .continueFromBackup(c): c.backup
        case let .continueFromDerive(c): c.backup
        case .continueFromInit, .complete: nil
        }
    }

    var body: some View {
        if let backup = availableBackup {
            saveBackupBody(backup: backup)
        } else {
            errorBody
        }
    }

    var errorBody: some View {
        VStack(spacing: 40) {
            header

            Spacer()

            VStack(spacing: 20) {
                Image(systemName: "x.circle.fill")
                    .font(.system(size: 100))
                    .foregroundStyle(.red)
                    .fontWeight(.light)

                Text("Could not complete setup")
                    .font(.title)
                    .fontWeight(.bold)

                Text(
                    "Please try again and hold your TAPSIGNER steady until setup is complete."
                )
                .font(.subheadline)
                .foregroundStyle(.primary.opacity(0.8))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal)

            Spacer()

            VStack(spacing: 14) {
                Button("Retry") { runContinueSetup() }
                    .disabled(isRunning)
                    .buttonStyle(DarkButtonStyle())
                    .padding(.horizontal)
            }
        }
        .background(patternBackground)
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }

    func saveBackupBody(backup: Data) -> some View {
        VStack(spacing: 32) {
            header

            Spacer()

            VStack(spacing: 20) {
                Image(systemName: "exclamationmark.shield.fill")
                    .font(.system(size: 100))
                    .foregroundStyle(.orange)
                    .fontWeight(.light)

                Text("Almost there")
                    .font(.title)
                    .fontWeight(.bold)

                Text(
                    "Your TAPSIGNER backup was created successfully, but setup didn't fully complete. Please download your backup now, then continue to finish setup."
                )
                .font(.subheadline)
                .foregroundStyle(.primary.opacity(0.8))
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal)

            ShareLink(
                item: BackupExport(
                    content: hexEncode(bytes: backup),
                    filename: "\(tapSigner.identFileNamePrefix())_backup.txt"
                ),
                preview: SharePreview("\(tapSigner.identFileNamePrefix())_backup.txt")
            ) {
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
                .clipShape(RoundedRectangle(cornerRadius: 10))
            }
            .font(.footnote)
            .fontWeight(.semibold)
            .padding(.horizontal)

            Spacer()

            VStack(spacing: 14) {
                Button("Continue") { runContinueSetup() }
                    .disabled(isRunning)
                    .buttonStyle(DarkButtonStyle())
                    .padding(.horizontal)
            }
        }
        .task {
            guard !didSaveBackup else { return }
            didSaveBackup = true
            let _ = app.saveTapSignerBackup(tapSigner, backup)
        }
        .background(patternBackground)
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }

    var header: some View {
        VStack {
            HStack {
                Button(action: { manager.popRoute() }) {
                    Text("Cancel")
                }

                Spacer()
            }
            .padding(.top, 20)
            .padding(.horizontal, 10)
            .foregroundStyle(.primary)
            .fontWeight(.semibold)
        }
    }

    var patternBackground: some View {
        VStack {
            Image(.chainCodePattern)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .ignoresSafeArea(edges: .all)
                .padding(.top, 5)

            Spacer()
        }
        .opacity(0.8)
    }

    func runContinueSetup() {
        guard !isRunning else { return }
        isRunning = true

        Task {
            let nfc = manager.getOrCreateNfc(tapSigner)
            switch await nfc.continueSetup(response) {
            case let .success(.complete(c)):
                manager.resetRoute(to: .setupSuccess(tapSigner, c))
            case let .success(incomplete):
                Log.error(
                    "Failed to complete TAPSIGNER setup, won't retry anymore \(incomplete)"
                )
                app.sheetState = nil
                app.alertState = .init(
                    .tapSignerSetupFailed(message: "Failed to set up TAPSIGNER")
                )
            case let .failure(error):
                app.sheetState = nil
                app.alertState = .init(.tapSignerSetupFailed(message: error.description))
            }

            isRunning = false
        }
    }
}

#Preview("Continue from backup") {
    TapSignerContainer(
        route:
        .setupRetry(
            tapSignerPreviewNew(preview: true),
            tapSignerSetupRetryContinueCmd(preview: true)
        )
    )
    .environment(AppManager.shared)
}
