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
    @State private var isSubmitting = false

    var body: some View {
        TapSignerAdaptiveLayout { usesFlexibleSpacing in
            TapSignerContinuationContent(
                usesFlexibleSpacing: usesFlexibleSpacing,
                title: "Setup in Progress",
                message: "Your TAPSIGNER saved its setup progress. Continue to finish setup.",
                isSubmitting: isSubmitting,
                cancelAction: cancel,
                continueAction: continueSetup
            )
        }
        .background(TapSignerResultBackground())
        .scrollIndicators(.hidden)
        .navigationBarHidden(true)
    }

    private func cancel() {
        app.sheetState = nil
    }

    private func continueSetup() {
        guard !isSubmitting else { return }

        isSubmitting = true

        Task {
            let nfc = manager.getOrCreateNfc(tapSigner)
            let result = await nfc.continueSetup(response)

            await MainActor.run {
                isSubmitting = false

                switch result {
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
