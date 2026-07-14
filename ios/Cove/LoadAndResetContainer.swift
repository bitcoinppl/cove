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
                    let generation = app.captureLoadAndResetGeneration()
                    async let minimumDelay: Void = Task.sleep(
                        for: .milliseconds(loadingTimeMs)
                    )
                    async let preparation = app.prepareLoadAndResetTarget(
                        generation: generation,
                        routes: nextRoute
                    )

                    let (_, outcome) = try await (minimumDelay, preparation)
                    guard case .ready = outcome else { return }

                    await app.resetAfterLoadingIfCurrent(
                        generation: generation,
                        route: route,
                        nextRoute: nextRoute
                    )
                } catch is CancellationError {
                    return
                } catch {
                    Log.error("Unable to prepare load-and-reset target: \(error)")
                }
            }
            .tint(.primary)
    }
}
