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

    var body: some View {
        VStack(spacing: 40) {
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
                Button("Retry") {
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
                                .tapSignerSetupFailed("Failed to setup TapSigner"))
                        case let .failure(error):
                            app.sheetState = nil
                            app.alertState = .init(.tapSignerSetupFailed(error.description))
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
        .setupRetry(
            tapSignerPreviewNew(preview: true),
            tapSignerSetupRetryContinueCmd(preview: true)
        )
    )
    .environment(AppManager.shared)
}
