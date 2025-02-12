//
//  QRCodeLabelImportView.swift
//  Cove
//
//  Created by Praveen Perera on 2/12/25.
//

import SwiftUI

struct QrCodeLabelImportView: View {
    @Environment(AppManager.self) var app
    @Environment(\.dismiss) private var dismiss

    // args
    @Binding var scannedCode: TaggedString?

    // private
    @State private var multiQr: MultiQr?
    @State private var scanComplete = false
    @State private var totalParts: Int? = nil
    @State private var partsLeft: Int? = nil
    @State private var showCameraAccessAlert = false

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

                        Text("Scan BIP329 Labels")
                            .font(.title2)
                            .foregroundStyle(.white)
                            .fontWeight(.semibold)

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
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .ignoresSafeArea(.all)
        .alert(isPresented: $showCameraAccessAlert) {
            Alert(
                title: Text("Camera Access Required"),
                message: Text(
                    "Please allow camera access in Settings to use this feature."),
                primaryButton: Alert.Button.default(Text("Settings")) {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                        app.popRoute()
                    }

                    let url = URL(string: UIApplication.openSettingsURLString)!
                    UIApplication.shared.open(url)
                },
                secondaryButton: Alert.Button.cancel {
                    Task {
                        await MainActor.run {
                            app.popRoute()
                        }
                    }
                }
            )
        }
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
                Log.error("error scanning bbqr part: \(error)")
            }

        case let .failure(error):
            if case ScanError.permissionDenied = error {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                    showCameraAccessAlert = true
                }
            }
        }
    }
}

#Preview {
    QrCodeLabelImportView(scannedCode: .constant(nil))
        .environment(AppManager.shared)
}
