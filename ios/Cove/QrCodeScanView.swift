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
    @Binding var alert: IdentifiableItem<AppAlertState>?
    @Binding var scannedCode: IdentifiableItem<StringOrData>?

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
        if case let .failure(error) = result {
            if case ScanError.permissionDenied = error {
                presentationMode.wrappedValue.dismiss()
                app.sheetState = .none

                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(1000)) {
                    alert = IdentifiableItem(AppAlertState.noCameraPermission)
                }
            }
        }

        guard case let .success(scanResult) = result else { return }
        let qr = StringOrData(scanResult.data)

        do {
            let multiQr: MultiQr =
                try self.multiQr
                    ?? {
                        let newMultiQr = try MultiQr.tryNew(qr: qr)
                        self.multiQr = newMultiQr
                        totalParts = Int(newMultiQr.totalParts())
                        return newMultiQr
                    }()

            // single QR
            if !multiQr.isBbqr() {
                scanComplete = true
                scannedCode = IdentifiableItem(qr)
                presentationMode.wrappedValue.dismiss()
                return
            }

            // BBQr
            guard case let .string(stringValue) = qr else { return }

            let result = try multiQr.addPart(qr: stringValue)
            partsLeft = Int(result.partsLeft())

            if result.isComplete() {
                scanComplete = true
                let data = try result.finalResult()
                scannedCode = IdentifiableItem(data)
                presentationMode.wrappedValue.dismiss()
            }
        } catch {
            Log.error("error scanning bbqr part: \(error)")
        }
    }
}

#Preview {
    struct PreviewContainer: View {
        @State private var app = MainViewModel()
        @State private var alert: IdentifiableItem<AppAlertState>? = nil
        @State private var scannedCode: IdentifiableItem<StringOrData>? = nil

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
