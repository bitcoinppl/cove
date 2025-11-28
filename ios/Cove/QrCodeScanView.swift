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
    @Binding var scannedCode: TaggedItem<MultiFormat>?

    // private
    @State private var scanner = QrScanner()
    @State private var scanComplete = false
    @State private var progress: ScanProgress? = nil

    var alertState: Binding<TaggedItem<AppAlertState>?> {
        $app.alertState
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
    }

    private func handleScan(result: Result<ScanResult, ScanError>) {
        // permission handling
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
            switch try scanner.scan(qr: qr) {
            case let .complete(data, _):
                scanComplete = true
                scannedCode = TaggedItem(data)
                scanner.reset()
                dismiss()

            case let .inProgress(prog):
                progress = prog
            }
        } catch {
            scanner.reset()
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
        @State private var scannedCode: TaggedItem<MultiFormat>? = nil

        var body: some View {
            QrCodeScanView(app: app, scannedCode: $scannedCode)
        }
    }

    return PreviewContainer()
}
