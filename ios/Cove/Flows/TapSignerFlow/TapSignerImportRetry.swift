//
//  TapSignerImportRetry.swift
//  Cove
//
//  Created by Praveen Perera on 3/25/25.
//

import SwiftUI
import UniformTypeIdentifiers

struct TapSignerImportRetry: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner
    let response: SetupCmdResponse

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
                Image(systemName: "x.circle.fill")
                    .font(.system(size: 100))
                    .foregroundStyle(.red)
                    .fontWeight(.light)

                Text("Couldn't complete setup")
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
                Button("Retry") {
                    Task {
                        switch await manager.nfc?.continueSetup(response) {
                        case let .success(.complete(c)):
                            manager.resetRoute(to: .importSuccess(tapSigner, c))
                        case let .success(incomplete):
                            Log.error("Failed to complete TAPSIGNER setup, won't retry anymore \(incomplete)")
                            app.sheetState = nil
                            app.alertState = .init(.tapSignerSetupFailed("Failed to setup TapSigner"))
                        case let .failure(error):
                            app.sheetState = nil
                            app.alertState = .init(.tapSignerSetupFailed(error.describe))
                        case .none:
                            app.sheetState = nil
                            app.alertState = .init(.tapSignerSetupFailed("Failed to get NFC reader"))
                        }
                    }
                }
                .buttonStyle(DarkButtonStyle())
                .padding(.horizontal)
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
    }
}

#Preview {
    TapSignerContainer(
        route:
        .importRetry(
            tapSignerPreviewNew(preview: true),
            tapSignerImportRetryContinueCmd(preview: true)
        )
    )
    .environment(AppManager.shared)
}
