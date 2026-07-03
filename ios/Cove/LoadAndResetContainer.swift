//
//  LoadAndResetContainer.swift
//  Cove
//
//  Created by Praveen Perera on 10/21/24.
//
import SwiftUI

struct LoadAndResetContainer: View {
    @Environment(AppManager.self) var app
    let route: Route
    let nextRoute: [Route]
    let loadingTimeMs: Int

    private var loadingTitle: String? {
        guard case .selectedWallet = nextRoute.first else { return nil }
        return String(localized: "Loading wallet...")
    }

    var body: some View {
        FullPageLoadingView(title: loadingTitle)
            .task(id: route) {
                do {
                    let generation = app.captureLoadAndResetGeneration()
                    app.startLoadAndResetTargetPrewarm(
                        generation: generation,
                        routes: nextRoute
                    )
                    try await Task.sleep(for: .milliseconds(loadingTimeMs))
                    app.resetAfterLoadingIfCurrent(
                        generation: generation,
                        route: route,
                        nextRoute: nextRoute
                    )
                } catch {}
            }
            .tint(.primary)
    }
}
