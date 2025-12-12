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
    @State private var density: QrDensity = .init()
    @State private var qrs: [QrCodeView] = []
    @State private var error: String? = nil
    @State private var currentIndex = 0

    let startedAt: Date = .now

    /// Animation interval: dynamic based on density for both formats
    var animationInterval: TimeInterval {
        switch selectedFormat {
        case .bbqr: Double(density.bbqrAnimationIntervalMs()) / 1000.0
        case .ur: Double(density.urAnimationIntervalMs()) / 1000.0
        }
    }

    var body: some View {
        VStack {
            Text("Scan this QR")
                .font(.title3)
                .padding(.top, 12)
                .fontWeight(.semibold)

            Text("Scan with your hardware wallet\nto sign your transaction")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.top, 1)
                .padding(.horizontal, 40)

            Picker("Format", selection: $selectedFormat) {
                ForEach(QrExportFormat.allCases, id: \.self) { format in
                    Text(String(describing: format)).tag(format)
                }
            }
            .pickerStyle(.segmented)
            .padding(.vertical, 8)
            .frame(maxWidth: 200)

            QrContent
        }
        .onChange(of: selectedFormat) { _, _ in
            generateQrCodes()
        }
        .onChange(of: density) { _, _ in
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
        VStack {
            // .id() forces TimelineView recreation when interval changes
            TimelineView(.periodic(from: startedAt, by: animationInterval)) { context in
                let index = abs(Int(context.date.distance(to: startedAt) / animationInterval) % qrs.count)
                qrs[index]
                    .frame(maxWidth: .infinity)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 11)
                    .onChange(of: index) { _, newIndex in
                        currentIndex = newIndex
                    }
            }
            .id(animationInterval)

            if qrs.count > 1 {
                HStack(alignment: .center, spacing: 8) {
                    MinusButtonMinimal
                    ProgressIndicator
                    PlusButtonMinimal
                }
                .padding(.horizontal, 9)
            } else {
                DensityButtons
                    .padding(.horizontal, 9)
            }
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
    }

    var canDecreaseDensity: Bool { density.canDecrease() }
    var canIncreaseDensity: Bool { density.canIncrease() && qrs.count > 1 }

    @ViewBuilder
    var MinusButtonMinimal: some View {
        Button { density = density.decrease() } label: {
            Image(systemName: "minus")
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(Color.secondary.opacity(canDecreaseDensity ? 1 : 0.3))
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .disabled(!canDecreaseDensity)
    }

    @ViewBuilder
    var PlusButtonMinimal: some View {
        Button { density = density.increase() } label: {
            Image(systemName: "plus")
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(Color.secondary.opacity(canIncreaseDensity ? 1 : 0.3))
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .disabled(!canIncreaseDensity)
    }

    @ViewBuilder
    var DensityButtons: some View {
        HStack(spacing: 0) {
            Button { density = density.decrease() } label: {
                Image(systemName: "minus")
                    .font(.system(size: 14, weight: .medium))
                    .frame(width: 32, height: 32)
                    .foregroundStyle(Color.secondary.opacity(canDecreaseDensity ? 1 : 0.3))
            }
            .disabled(!canDecreaseDensity)

            Divider()
                .frame(height: 20)

            Button { density = density.increase() } label: {
                Image(systemName: "plus")
                    .font(.system(size: 14, weight: .medium))
                    .frame(width: 32, height: 32)
                    .foregroundStyle(Color.secondary.opacity(canIncreaseDensity ? 1 : 0.3))
            }
            .disabled(!canIncreaseDensity)
        }
        .background(Color.secondary.opacity(0.15))
        .cornerRadius(50)
    }

    func generateQrCodes() {
        do {
            let strings: [String] = switch selectedFormat {
            case .bbqr:
                try details.psbtToBbqrWithDensity(density: density)
            case .ur:
                try details.psbtToUrWithDensity(density: density)
            }
            qrs = strings.map { QrCodeView(text: $0) }
            currentIndex = 0
            error = nil
        } catch let err {
            error = err.localizedDescription
            qrs = []
        }
    }
}

#Preview {
    AsyncPreview {
        SendFlowQrExport(details: confirmDetailsPreviewNew())
            .padding()
    }
}

#Preview("Sheet - Multi QR") {
    struct SheetPreview: View {
        @State private var isPresented = true

        var body: some View {
            VStack {
                Button("Show Sheet") {
                    isPresented = true
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.midnightBlue.edgesIgnoringSafeArea(.all))
            .sheet(isPresented: $isPresented) {
                SendFlowQrExport(details: confirmDetailsPreviewNew())
                    .presentationDetents([.height(550), .height(650), .large])
                    .padding()
                    .padding(.top, 10)
            }
        }
    }

    return AsyncPreview {
        SheetPreview()
    }
}
