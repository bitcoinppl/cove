//
//  KeyTeleportReceiveUnavailableView.swift
//  Cove
//

import SwiftUI

struct KeyTeleportReceiveUnavailableView: View {
    @Environment(AppManager.self) private var app

    var body: some View {
        VStack(spacing: 20) {
            Spacer()

            Image(systemName: "arrow.triangle.2.circlepath")
                .font(.system(size: 60))
                .foregroundStyle(.secondary)

            Text("Key Teleport")
                .font(.title2)
                .fontWeight(.semibold)

            Text("Key Teleport receive is not yet available on iOS. Please use the Android app to receive a wallet via Key Teleport.")
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)

            Spacer()

            Button("Dismiss") {
                app.popRoute()
            }
            .buttonStyle(.borderedProminent)
            .padding(.bottom, 32)
        }
    }
}
