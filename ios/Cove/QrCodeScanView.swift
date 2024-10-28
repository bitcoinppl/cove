//
//  QrCodeScanView.swift
//  Cove
//
//  Created by Praveen Perera on 10/27/24.
//

import SwiftUI

struct QrCodeScanView: View {
    @Environment(\.presentationMode) var presentationMode

    // public
    @Binding var app: MainViewModel
    @Binding var alert: PresentableItem<AppAlertState>?
    @Binding var scannedCode: IdentifiableString?

    // private
    @State private var multiQr: MultiQr?

    // bbqr
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
                VStack {
                    ZStack {
                        RoundedRectangle(cornerRadius: 20)
                            .stroke(Color.primary, lineWidth: 3)
                            .frame(height: qrCodeHeight)

                        ScannerView(
                            codeTypes: [.qr],
                            scanMode: .oncePerCode,
                            scanInterval: 0.1,
                            showAlert: false,
                            completion: handleScan
                        )
                        .frame(height: qrCodeHeight)
                        .clipShape(RoundedRectangle(cornerRadius: 18))
                    }
                    .padding(.horizontal)

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
                }
                .padding(.top, 18)
                .padding(.bottom, 36)
            }
        }
        .padding()
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
            if multiQr.isSingle() {
                scanComplete = true
                scannedCode = IdentifiableString(stringValue)
                return
            }

            // BBQr
            do {
                let result = try multiQr.addPart(qr: stringValue)
                partsLeft = Int(result.partsLeft())

                if result.isComplete() {
                    scanComplete = true
                    let data = try result.finalResult()
                    scannedCode = IdentifiableString(data)
                }
            } catch {
                Log.error("error scanning bbqr part: \(error)")
            }

        case let .failure(error):
            if case ScanError.permissionDenied = error {
                DispatchQueue.main.async {}
            }
        }
    }
}

#Preview {
    struct PreviewContainer: View {
        @State private var app = MainViewModel()
        @State private var alert: PresentableItem<AppAlertState>? = nil
        @State private var scannedCode: IdentifiableString? = nil

        var body: some View {
            QrCodeScanView(
                app: $app,
                alert: $alert,
                scannedCode: $scannedCode
            )
        }
    }

    return PreviewContainer()
}
