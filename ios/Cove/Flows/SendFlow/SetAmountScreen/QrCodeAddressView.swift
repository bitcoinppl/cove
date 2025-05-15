//
//  QrCodeAddressView.swift
//  Cove
//
//  Created by Praveen Perera on 11/7/24.
//

import SwiftUI

struct QrCodeAddressView: View {
    @State private var multiQr: MultiQr?
    @Environment(AppManager.self) var app
    @Environment(\.dismiss) private var dismiss

    // passed in
    @Binding var scannedCode: TaggedString?

    // private
    @State private var scanComplete = false
    @State private var totalParts: Int? = nil
    @State private var partsLeft: Int? = nil

    private let screenHeight = UIScreen.main.bounds.height

    var partsScanned: Int {
        if let totalParts, let partsLeft {
            totalParts - partsLeft
        } else {
            0
        }
    }

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

                        if let totalParts, let partsLeft {
                            Text("Scanned \(partsScanned) of \(totalParts)")
                                .font(.subheadline)
                                .fontWeight(.medium)
                                .padding(.top, 8)

                            Text("\(partsLeft) parts left")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .fontWeight(.bold)
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
        case let .success(result):
            guard case let .string(stringValue) = result.data else { return }

            if multiQr == nil {
                multiQr = MultiQr.newFromString(qr: stringValue)
                totalParts = Int(multiQr?.totalParts() ?? 0)
            }

            guard let multiQr else { return }

            // single QR
            if !multiQr.isBbqr() {
                scanComplete = true
                scannedCode = TaggedString(stringValue)
                return
            }

            // BBQr
            do {
                let result = try multiQr.addPart(qr: stringValue)
                partsLeft = Int(result.partsLeft())

                if result.isComplete() {
                    scanComplete = true
                    let data = try result.finalResult()
                    scannedCode = TaggedString(data)
                }
            } catch {
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
