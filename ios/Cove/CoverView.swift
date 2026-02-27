//
//  CoverView.swift
//  Cove
//
//  Created by Praveen Perera on 12/15/24.
//

import SwiftUI

struct CoverView: View {
    /// Delay before showing the loading spinner, in milliseconds.
    /// Prevents a distracting spinner flash when bootstrap completes quickly
    static let spinnerDelayMs = 100

    var errorMessage: String? = nil

    var body: some View {
        ZStack {
            Color.black.edgesIgnoringSafeArea(.all)

            if let errorMessage {
                StorageErrorView(errorMessage: errorMessage)
            } else {
                SplashLoadingView()
            }
        }
    }
}

private struct StorageErrorView: View {
    let errorMessage: String

    var body: some View {
        VStack(spacing: 8) {
            Text("Storage Error")
                .font(.headline)
                .foregroundColor(.white)
            Text(errorMessage)
                .font(.subheadline)
                .foregroundColor(.white.opacity(0.7))
                .multilineTextAlignment(.center)
                .padding(.horizontal, 16)
            Text("Please contact feedback@covebitcoinwallet.com for help")
                .font(.caption)
                .foregroundColor(.white.opacity(0.5))
                .padding(.top, 8)
        }
    }
}

private struct SplashLoadingView: View {
    @State private var showSpinner = false

    var body: some View {
        VStack(spacing: 24) {
            Image(.icon)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(width: 144, height: 144)
                .cornerRadius(25.263)

            if showSpinner {
                ProgressView()
                    .tint(.white)
            }
        }
        .task {
            try? await Task.sleep(for: .milliseconds(CoverView.spinnerDelayMs))
            showSpinner = true
        }
    }
}

#Preview {
    CoverView()
}
