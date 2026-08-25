//
//  TapSignerSetupRetryView.swift
//  Cove
//
//  Created by Praveen Perera on 3/25/25.
//

import CoveCore
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
        app.sheetState = nil
    }

    private func retry() {
        Task {
            let nfc = manager.getOrCreateNfc(tapSigner)

            switch await nfc.continueSetup(response) {
            case let .success(.complete(complete)):
                manager.resetRoute(to: .setupSuccess(tapSigner, complete))
            case let .success(.retry(next)):
                manager.resetRoute(to: .setupRetry(tapSigner, .retry(next)))
            case .failure:
                app.sheetState = nil
                app.alertState = .init(
                    .tapSignerSetupFailed(
                        message: "TapSigner setup failed. Please try again."
                    )
                )
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
