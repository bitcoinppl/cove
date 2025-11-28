//
//  QrCodeAddressView.swift
//  Cove
//
//  Created by Praveen Perera on 11/7/24.
//

import SwiftUI

struct QrCodeAddressView: View {
    @State private var scanner = QrScanner()
    @Environment(AppManager.self) var app
    @Environment(\.dismiss) private var dismiss

    // passed in
    @Binding var scannedCode: TaggedString?

    // private
    @State private var scanComplete = false
    @State private var progress: ScanProgress? = nil

    private let screenHeight = UIScreen.main.bounds.height

    var qrCodeHeight: CGFloat {
        screenHeight * 0.6
    }

    var body: some View {
        VStack {
            if !scanComplete {
                ZStack {
                    ScannerView(
                        codeTypes: [.qr],
                        scanMode: .oncePerCode,
                        scanInterval: 0.1,
                        showAlert: false,
                        completion: handleScan
                    )
                    .ignoresSafeArea(.all)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                    VStack {
                        Spacer()

                        VStack(spacing: 12) {
                            Text("Scan Wallet Address")
                                .font(.title2)
                                .foregroundStyle(.white)
                                .fontWeight(.bold)

                            Text(
                                "Effortlessly send Bitcoin—scan the recipient’s QR code to get their address"
                            )
                            .foregroundStyle(.white)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 8)
                            .fontWeight(.semibold)
                        }

                        Spacer()
                        Spacer()
                        Spacer()
                        Spacer()
                        Spacer()

                        if let progress {
                            VStack(spacing: 8) {
                                Text(progress.displayText())
                                    .font(.subheadline)
                                    .fontWeight(.medium)
                                    .padding(.top, 8)

                                if let detailText = progress.detailText() {
                                    Text(detailText)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                        .fontWeight(.bold)
                                }
                            }
                        }

                        Spacer()
                    }
                    .safeAreaPadding(.all)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.black.opacity(0.20))
    }

    private func handleScan(result: Result<ScanResult, ScanError>) {
        switch result {
        case let .success(scanResult):
            let qr = StringOrData(scanResult.data)

            do {
                switch try scanner.scan(qr: qr) {
                case let .complete(_, rawData):
                    scanComplete = true
                    if let raw = rawData {
                        scannedCode = TaggedString(raw)
                    } else if case let .string(str) = scanResult.data {
                        scannedCode = TaggedString(str)
                    }
                    scanner.reset()

                case let .inProgress(prog):
                    progress = prog
                }
            } catch {
                dismiss()
                app.alertState = TaggedItem(
                    .general(
                        title: "QR Scan Error",
                        message: "Unable to scan QR code, error: \(error.localizedDescription)"
                    ))
            }

        case let .failure(error):
            if case ScanError.permissionDenied = error {
                dismiss()
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(1100)) {
                    app.alertState = TaggedItem(.noCameraPermission)
                }
            }
        }
    }
}

#Preview {
    VStack {
        QrCodeAddressView(scannedCode: Binding.constant(nil))
            .environment(AppManager.shared)
    }
    .ignoresSafeArea(.all)
    .frame(maxWidth: .infinity, maxHeight: .infinity)
}
