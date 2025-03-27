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

                Text("Could not complete import")
                    .font(.title)
                    .fontWeight(.bold)

                Text(
                    "Please try again and hold your TAPSIGNER steady until import is complete."
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
                    guard let pin = manager.enteredPin else {
                        app.alertState = .init(.tapSignerDeriveFailed("No PIN entered"))
                        return
                    }

                    let nfc = TapSignerNFC(tapSigner)
                    manager.nfc = nfc

                    Task {
                        switch await nfc.derive(pin: pin) {
                        case let .success(deriveInfo):
                            manager.resetRoute(to: .importSuccess(tapSigner, deriveInfo))
                        case let .failure(error):
                            app.alertState = .init(.tapSignerDeriveFailed(error.describe))
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
        route: .importRetry(tapSignerPreviewNew(preview: true))
    )
    .environment(AppManager.shared)
}
