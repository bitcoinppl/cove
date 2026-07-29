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
        TapSignerAdaptiveLayout { usesFlexibleSpacing in
            TapSignerRetryContent(
                usesFlexibleSpacing: usesFlexibleSpacing,
                title: "Could not complete setup",
                message: """
                Please try again and hold your TAPSIGNER steady until setup is complete.
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
        manager.popRoute()
    }

    private func retry() {
        Task {
            let nfc = manager.getOrCreateNfc(tapSigner)

            switch await nfc.continueSetup(response) {
            case let .success(.complete(complete)):
                manager.resetRoute(to: .setupSuccess(tapSigner, complete))
            case let .success(incomplete):
                Log.error(
                    "Failed to complete TAPSIGNER setup, won't retry anymore \(incomplete)"
                )
                app.sheetState = nil
                app.alertState = .init(
                    .tapSignerSetupFailed(message: "Failed to setup TapSigner")
                )
            case let .failure(error):
                app.sheetState = nil
                app.alertState = .init(.tapSignerSetupFailed(message: error.description))
            }
        }
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
