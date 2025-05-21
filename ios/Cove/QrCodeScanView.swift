//
//  QrCodeScanView.swift
//  Cove
//
//  Created by Praveen Perera on 10/27/24.
//

import SwiftUI

struct QrCodeScanView: View {
    @Environment(\.dismiss) private var dismiss

    // public
    @Bindable var app: AppManager
    @Binding var scannedCode: TaggedItem<StringOrData>?

    // private
    @State private var multiQr: MultiQr?

    // bbqr
    @State private var scanComplete = false
    @State private var totalParts: Int? = nil
    @State private var partsLeft: Int? = nil

    var alertState: Binding<TaggedItem<AppAlertState>?> {
        $app.alertState
    }

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
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .ignoresSafeArea(.all)

                    VStack {
                        Spacer()
                        Spacer()
                        Spacer()
                        Spacer()
                        Spacer()
                        Spacer()

                        if let totalParts, let partsLeft {
                            Group {
                                Text("Scanned \(partsScanned) of \(totalParts)")
                                    .font(.subheadline)
                                    .fontWeight(.medium)
                                    .padding(.top, 8)

                                Text("\(partsLeft) parts left")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .fontWeight(.bold)
                            }
                            .foregroundStyle(.white)
                        }

                        Spacer()
                    }
                }
            }
        }
    }

    private func handleScan(result: Result<ScanResult, ScanError>) {
        if case let .failure(error) = result {
            if case ScanError.permissionDenied = error {
                dismiss()
                app.sheetState = .none

                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(1000)) {
                    app.alertState = TaggedItem(AppAlertState.noCameraPermission)
                }
            }
        }

        guard case let .success(scanResult) = result else { return }
        let qr = StringOrData(scanResult.data)

        do {
            let multiQr: MultiQr =
                try multiQr
                    ?? {
                        let newMultiQr = try MultiQr.tryNew(qr: qr)
                        self.multiQr = newMultiQr
                        totalParts = Int(newMultiQr.totalParts())
                        return newMultiQr
                    }()

            // single QR
            if !multiQr.isBbqr() {
                scanComplete = true
                scannedCode = TaggedItem(qr)
                dismiss()
                return
            }

            // BBQr
            guard case let .string(stringValue) = qr else { return }

            let result = try multiQr.addPart(qr: stringValue)
            partsLeft = Int(result.partsLeft())

            if result.isComplete() {
                scanComplete = true
                let data = try result.finalResult()
                scannedCode = TaggedItem(data)
                dismiss()
            }
        } catch {
            dismiss()
            app.alertState = TaggedItem(
                .general(
                    title: "QR Scan Error",
                    message: "Unable to scan QR code, error: \(error.localizedDescription)"
                ))
        }
    }
}

#Preview {
    struct PreviewContainer: View {
        @State private var app = AppManager.shared
        @State private var alert: TaggedItem<AppAlertState>? = nil
        @State private var scannedCode: TaggedItem<StringOrData>? = nil

        var body: some View {
            QrCodeScanView(app: app, scannedCode: $scannedCode)
        }
    }

    return PreviewContainer()
}
