//
//  QrCodeLabelImportView.swift
//  Cove
//
//  Created by Praveen Perera on 2/12/25.
//

import SwiftUI
import UIKit

struct QrCodeLabelImportView: View {
    @Environment(AppManager.self) var app
    @Environment(\.dismiss) private var dismiss

    // args
    @Binding var scannedCode: TaggedItem<MultiFormat>?

    // private
    @State private var scanner = QrScanner()
    @State private var scanComplete = false
    @State private var progress: ScanProgress? = nil
    @State private var showCameraAccessAlert = false

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
        case let .success(scanResult):
            let qr = StringOrData(scanResult.data)

            do {
                switch try scanner.scan(qr: qr) {
                case let .complete(data, haptic):
                    haptic.trigger()
                    scanComplete = true
                    scannedCode = TaggedItem(data)
                    scanner.reset()

                case let .inProgress(prog, haptic):
                    haptic.trigger()
                    progress = prog
                }
            } catch {
                scanner.reset()
                app.alertState = TaggedItem(
                    .general(
                        title: "QR Scan Error",
                        message: "Unable to scan QR code, error: \(error.localizedDescription)"
                    )
                )
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
