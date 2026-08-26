//
//  TapSignerImportRetryView.swift
//  Cove
//
//  Created by Praveen Perera on 3/25/25.
//

import CoveCore
import SwiftUI
import UniformTypeIdentifiers

struct TapSignerImportRetry: View {
    @Environment(AppManager.self) private var app
    @Environment(TapSignerManager.self) private var manager

    let tapSigner: TapSigner

    var body: some View {
        TapSignerAdaptiveLayout { usesFlexibleSpacing in
            TapSignerRetryContent(
                usesFlexibleSpacing: usesFlexibleSpacing,
                title: "Could not complete import",
                message: """
                Please try again and hold your TAPSIGNER steady until import is complete.
                """,
                cancelAction: cancel,
                retryAction: retry
            )
        }
        .background(TapSignerResultBackground())
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }

    private func cancel() {
        manager.cancel()
        app.sheetState = nil
    }

    private func retry() {
        guard let pin = manager.enteredPin else {
            app.alertState = .init(.tapSignerDeriveFailed(message: "No PIN entered"))
            return
        }

        let nfc = manager.getOrCreateNfc(tapSigner)

        Task {
            switch await nfc.derive(pin: pin) {
            case let .success(deriveInfo):
                manager.enteredPin = nil
                manager.resetRoute(to: .importSuccess(tapSigner, deriveInfo))
            case let .failure(error):
                if error.isAuthError() {
                    app.sheetState = nil
                    app.alertState = .init(
                        .tapSignerWrongPin(tapSigner: tapSigner, action: .derive)
                    )
                } else {
                    app.alertState = .init(
                        .tapSignerDeriveFailed(
                            message: "TapSigner import failed. Please try again."
                        )
                    )
                }
            }
        }
    }
}

#Preview {
    TapSignerContainer(
        route: .importRetry(tapSignerPreviewNew(preview: true))
    )
    .environment(AppManager.shared)
}
