//
//  QrExportView.swift
//  Cove
//
//  Created by Praveen Perera on 11/24/24.
//
import SwiftUI

extension QrExportFormat: CaseIterable {
    public static var allCases: [QrExportFormat] {
        [.bbqr, .ur]
    }
}

/// Generic QR export view that can display animated BBQr or UR QR codes
/// If `generateUrStrings` is nil, the format picker is hidden and only BBQr is used
struct QrExportView: View {
    let title: String
    let subtitle: String
    let generateBbqrStrings: (QrDensity) async throws -> [String]
    let generateUrStrings: ((QrDensity) async throws -> [String])?
    let copyData: (() async throws -> String)?

    @State private var selectedFormat: QrExportFormat = .bbqr
    @State private var density: QrDensity = .init()
    @State private var qrs: [QrCodeView] = []
    @State private var error: String? = nil
    @State private var currentIndex = 0
    @State private var startedAt = Date()

    /// Whether to show the format picker (only if UR is available)
    var showFormatPicker: Bool {
        generateUrStrings != nil
    }

    /// Animation interval: dynamic based on density for both formats
    var animationInterval: TimeInterval {
        switch selectedFormat {
        case .bbqr: Double(density.bbqrAnimationIntervalMs()) / 1000.0
        case .ur: Double(density.urAnimationIntervalMs()) / 1000.0
        }
    }

    var body: some View {
        VStack {
            HStack {
                Spacer()
                Text(title)
                    .font(.title3)
                    .fontWeight(.semibold)
                Spacer()
            }
            .overlay(alignment: .trailing) {
                if copyData != nil {
                    Button {
                        Task { await copyToClipboard() }
                    } label: {
                        Image(systemName: "doc.on.doc")
                            .font(.body)
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .padding(.trailing, 4)
                }
            }
            .padding(.top, 12)

            Text(subtitle)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .lineLimit(nil)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 1)
                .padding(.horizontal, 40)

            if showFormatPicker {
                Picker("Format", selection: $selectedFormat) {
                    ForEach(QrExportFormat.allCases, id: \.self) { format in
                        Text(String(describing: format)).tag(format)
                    }
                }
                .pickerStyle(.segmented)
                .padding(.vertical, 8)
                .frame(maxWidth: 200)
            }

            QrExportContent(
                error: error,
                qrs: qrs,
                currentIndex: $currentIndex,
                density: $density,
                startedAt: startedAt,
                animationInterval: animationInterval
            )
        }
        .onChange(of: selectedFormat) { _, _ in
            Task { await generateQrCodes() }
        }
        .onChange(of: density) { _, _ in
            Task { await generateQrCodes() }
        }
        .task {
            await generateQrCodes()
        }
    }

    func generateQrCodes() async {
        do {
            let strings: [String] = switch selectedFormat {
            case .bbqr:
                try await generateBbqrStrings(density)
            case .ur:
                if let generateUrStrings {
                    try await generateUrStrings(density)
                } else {
                    // fallback to BBQr if UR not available
                    try await generateBbqrStrings(density)
                }
            }
            qrs = strings.map { QrCodeView(text: $0) }
            currentIndex = 0
            error = nil
        } catch let err {
            error = err.localizedDescription
            qrs = []
        }
    }

    func copyToClipboard() async {
        guard let copyData else { return }
        do {
            let data = try await copyData()
            UIPasteboard.general.string = data
            await FloaterPopup(text: "Copied").dismissAfter(2).present()
        } catch {
            Log.error("Failed to copy data: \(error)")
        }
    }
}

private struct QrExportContent: View {
    let error: String?
    let qrs: [QrCodeView]
    @Binding var currentIndex: Int
    @Binding var density: QrDensity
    let startedAt: Date
    let animationInterval: TimeInterval

    var body: some View {
        if let error {
            Text(error)
                .font(.footnote)
                .foregroundStyle(.red)
                .padding(.top, 8)
        } else if qrs.isEmpty {
            ProgressView()
                .padding(.top, 20)
        } else {
            QrExportAnimatedQrView(
                qrs: qrs,
                currentIndex: $currentIndex,
                density: $density,
                startedAt: startedAt,
                animationInterval: animationInterval
            )
        }
    }
}

private struct QrExportAnimatedQrView: View {
    let qrs: [QrCodeView]
    @Binding var currentIndex: Int
    @Binding var density: QrDensity
    let startedAt: Date
    let animationInterval: TimeInterval

    var body: some View {
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
                    QrExportDensityButton(
                        systemImage: "minus",
                        size: 44,
                        isEnabled: canDecreaseDensity,
                        action: decreaseDensity
                    )

                    QrExportProgressIndicator(qrCount: qrs.count, currentIndex: currentIndex)

                    QrExportDensityButton(
                        systemImage: "plus",
                        size: 44,
                        isEnabled: canIncreaseDensity,
                        action: increaseDensity
                    )
                }
                .padding(.horizontal, 9)
            } else {
                QrExportDensityButtons(
                    canDecrease: canDecreaseDensity,
                    canIncrease: canIncreaseDensity,
                    decrease: decreaseDensity,
                    increase: increaseDensity
                )
                .padding(.horizontal, 9)
            }
        }
    }

    private var canDecreaseDensity: Bool {
        density.canDecrease()
    }

    private var canIncreaseDensity: Bool {
        density.canIncrease() && qrs.count > 1
    }

    private func decreaseDensity() {
        density = density.decrease()
    }

    private func increaseDensity() {
        density = density.increase()
    }
}

private struct QrExportProgressIndicator: View {
    let qrCount: Int
    let currentIndex: Int

    var body: some View {
        HStack(spacing: 4) {
            ForEach(0 ..< qrCount, id: \.self) { index in
                Rectangle()
                    .fill(Color.blue)
                    .opacity(index == currentIndex ? 1 : 0.3)
                    .frame(height: 12)
                    .cornerRadius(2)
            }
        }
    }
}

private struct QrExportDensityButton: View {
    let systemImage: String
    let size: CGFloat
    let isEnabled: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(Color.secondary.opacity(isEnabled ? 1 : 0.3))
                .frame(width: size, height: size)
                .contentShape(Rectangle())
        }
        .disabled(!isEnabled)
    }
}

private struct QrExportDensityButtons: View {
    let canDecrease: Bool
    let canIncrease: Bool
    let decrease: () -> Void
    let increase: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            QrExportDensityButton(
                systemImage: "minus",
                size: 32,
                isEnabled: canDecrease,
                action: decrease
            )

            Divider()
                .frame(height: 20)

            QrExportDensityButton(
                systemImage: "plus",
                size: 32,
                isEnabled: canIncrease,
                action: increase
            )
        }
        .background(Color.secondary.opacity(0.15))
        .cornerRadius(50)
    }
}

// MARK: - Convenience initializer for PSBT export (backwards compatibility)

extension QrExportView {
    /// Convenience initializer for PSBT export with ConfirmDetails
    init(details: ConfirmDetails) {
        self.init(
            title: "Scan this QR",
            subtitle: "Scan with your hardware wallet\nto sign your transaction",
            generateBbqrStrings: { density in try details.psbtToBbqrWithDensity(density: density) },
            generateUrStrings: { density in try details.psbtToUrWithDensity(density: density) },
            copyData: { details.psbtToHex() }
        )
    }
}

#Preview {
    AsyncPreview {
        QrExportView(details: confirmDetailsPreviewNew())
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
                QrExportView(details: confirmDetailsPreviewNew())
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
