//
//  LoadAndResetContainer.swift
//  Cove
//
//  Created by Praveen Perera on 10/21/24.
//
import SwiftUI

struct LoadAndResetContainer: View {
    @Environment(AppManager.self) var app
    let nextRoute: [Route]
    let loadingTimeMs: Int

    var body: some View {
        ProgressView()
            .task {
                do {
                    try await Task.sleep(for: .milliseconds(loadingTimeMs))
                    app.rust.resetAfterLoading(to: nextRoute)
                } catch {}
            }
            .tint(.primary)
    }
}
