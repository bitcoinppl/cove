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

    var body: some View {
        ProgressView()
            .task(id: route) {
                do {
                    let generation = await app.captureLoadAndResetGeneration()
                    try await Task.sleep(for: .milliseconds(loadingTimeMs))
                    await app.resetAfterLoadingIfCurrent(
                        generation: generation,
                        route: route,
                        nextRoute: nextRoute
                    )
                } catch {}
            }
            .tint(.primary)
    }
}
