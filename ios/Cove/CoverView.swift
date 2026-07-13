//
//  CoverView.swift
//  Cove
//
//  Created by Praveen Perera on 12/15/24.
//

import SwiftUI
import UIKit

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
    @State private var copiedDiagnostics = false

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

            HStack(spacing: 12) {
                Button(copiedDiagnostics ? "Copied" : "Copy Diagnostics") {
                    UIPasteboard.general.string = StartupDiagnostics.report(errorMessage: errorMessage)
                    copiedDiagnostics = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                        copiedDiagnostics = false
                    }
                }
                .buttonStyle(.bordered)

                Button("Share Diagnostics") {
                    ShareSheet.present(
                        data: StartupDiagnostics.report(errorMessage: errorMessage),
                        filename: StartupDiagnostics.filename
                    ) { success in
                        if !success {
                            Log.warn("Startup diagnostics share cancelled or failed")
                        }
                    }
                }
                .buttonStyle(.bordered)
            }
            .font(.caption)
            .tint(.white)
            .padding(.top, 12)
        }
    }
}

private enum StartupDiagnostics {
    static let filename = "cove-startup-diagnostics.txt"

    static func report(errorMessage: String) -> String {
        [
            "Cove startup diagnostics",
            "Generated: \(ISO8601DateFormatter().string(from: Date()))",
            "",
            "App",
            "Version: \(bundleValue("CFBundleShortVersionString"))",
            "Build: \(bundleValue("CFBundleVersion"))",
            "iOS: \(UIDevice.current.systemVersion)",
            "Device: \(UIDevice.current.model)",
            "",
            "Platform error",
            errorMessage,
            "",
            startupDiagnosticTextReport(),
        ].joined(separator: "\n")
    }

    private static func bundleValue(_ key: String) -> String {
        Bundle.main.object(forInfoDictionaryKey: key) as? String ?? "unknown"
    }
}

private struct SplashLoadingView: View {
    @State private var showSpinner = false
    @State private var statusMessage: String? = nil
    @State private var encryptionProgress: Double? = nil

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

            if let statusMessage {
                Text(statusMessage)
                    .font(.subheadline)
                    .foregroundColor(.white.opacity(0.7))
            }

            if let encryptionProgress {
                ProgressView(value: encryptionProgress)
                    .progressViewStyle(.linear)
                    .tint(.white)
                    .frame(width: 200)
            }
        }
        .task {
            try? await Task.sleep(for: .milliseconds(CoverView.spinnerDelayMs))
            showSpinner = true
        }
        .task {
            while !Task.isCancelled {
                do { try await Task.sleep(for: .milliseconds(66)) }
                catch { break }

                if let progress = activeMigration()?.progress(),
                   progress.total > 0
                {
                    statusMessage = "Encrypting data..."
                    encryptionProgress =
                        Double(progress.current) / Double(progress.total)
                } else if encryptionProgress != nil {
                    statusMessage = nil
                    encryptionProgress = nil
                }
            }
        }
    }
}

#Preview {
    CoverView()
}
