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
                try? await Task.sleep(for: .milliseconds(loadingTimeMs))
                app.resetRoute(to: nextRoute)
            }
            .tint(.primary)
    }
}

#Preview {
    LoadAndResetContainer(nextRoute: [.listWallets], loadingTimeMs: 100).environment(AppManager())
}
