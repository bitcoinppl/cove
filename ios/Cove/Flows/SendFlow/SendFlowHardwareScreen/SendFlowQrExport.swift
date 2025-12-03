//
//  SendFlowQrExport.swift
//  Cove
//
//  Created by Praveen Perera on 11/24/24.
//
import SwiftUI

extension QrExportFormat: CaseIterable {
    public static var allCases: [QrExportFormat] { [.bbqr, .ur] }
}

struct SendFlowQrExport: View {
    let details: ConfirmDetails

    @State private var selectedFormat: QrExportFormat = .bbqr
    @State private var qrs: [QrCodeView] = []
    @State private var error: String? = nil
    @State private var currentIndex = 0

    let startedAt: Date = .now
    let every: TimeInterval = 0.250

    var body: some View {
        VStack {
            Picker("Format", selection: $selectedFormat) {
                ForEach(QrExportFormat.allCases, id: \.self) { format in
                    Text(String(describing: format)).tag(format)
                }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 40)

            Text("Scan this QR")
                .font(.headline)
                .padding(.top, 12)

            Text("Scan with your hardware wallet to sign your transaction")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.top, 2)
                .padding(.horizontal, 40)

            QrContent
        }
        .onChange(of: selectedFormat) { _, _ in
            generateQrCodes()
        }
        .onAppear {
            generateQrCodes()
        }
    }

    @ViewBuilder
    var QrContent: some View {
        if let error {
            Text(error)
                .font(.footnote)
                .foregroundStyle(.red)
                .padding(.top, 8)
        } else if qrs.isEmpty {
            ProgressView()
                .padding(.top, 20)
        } else {
            AnimatedQrView
        }
    }

    @ViewBuilder
    var AnimatedQrView: some View {
        TimelineView(.periodic(from: startedAt, by: every)) { context in
            let index = abs(Int(context.date.distance(to: startedAt) / every) % qrs.count)
            qrs[index]
                .onChange(of: index) { _, newIndex in
                    currentIndex = newIndex
                }
        }

        if qrs.count > 1 {
            ProgressIndicator
        }
    }

    @ViewBuilder
    var ProgressIndicator: some View {
        HStack(spacing: 4) {
            ForEach(0 ..< qrs.count, id: \.self) { index in
                Rectangle()
                    .fill(Color.blue)
                    .opacity(index == currentIndex ? 1 : 0.3)
                    .frame(height: 12)
                    .cornerRadius(2)
            }
        }
        .padding(.top, 20)
    }

    func generateQrCodes() {
        do {
            let strings: [String] = switch selectedFormat {
            case .bbqr:
                try details.psbtToBbqr()
            case .ur:
                try details.psbtToUr(maxFragmentLen: 200)
            }
            qrs = strings.map { QrCodeView(text: $0) }
            error = nil
        } catch let err {
            error = err.localizedDescription
            qrs = []
        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowQrExport(details: ConfirmDetails.previewNew())
            .padding()
    }
}
