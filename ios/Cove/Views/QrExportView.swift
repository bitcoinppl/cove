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

/// The frames an export animates through
enum QrExportFrames {
    /// A fixed set shown in a loop, as a BBQr split is
    case loop([String])
    /// An animated UR, which mixes fountain parts in with its plain fragments after the first
    /// pass, so a scanner that keeps missing a frame completes from a mixed part instead
    case fountain(UrFountain)

    /// The frames in one pass, which the frame counter shows
    var passCount: Int {
        switch self {
        case let .loop(frames): frames.count
        case let .fountain(fountain): Int(fountain.sequenceCount())
        }
    }

    /// The frame to show on the animation's `tick`th tick, counting from 0
    func frame(at tick: Int) -> String {
        switch self {
        case let .loop(frames): frames[tick % frames.count]
        case let .fountain(fountain): fountain.frame(index: UInt32(truncatingIfNeeded: tick))
        }
    }

    /// Where in one pass the frame counter stands on the animation's `tick`th tick
    func passPosition(at tick: Int) -> Int {
        switch self {
        case let .loop(frames): tick % frames.count
        case let .fountain(fountain):
            Int(fountain.passPosition(index: UInt32(truncatingIfNeeded: tick)))
        }
    }
}

/// Generic QR export view that can display animated BBQr or UR QR codes
/// If `generateUr` is nil, the format picker is hidden and only BBQr is used
struct QrExportView: View {
    let title: String
    let subtitle: String
    let generateBbqrStrings: (QrDensity) async throws -> [String]
    let generateUr: ((QrDensity) async throws -> UrFountain)?
    let copyData: (() async throws -> String)?

    @State private var selectedFormat: QrExportFormat = .bbqr
    @State private var density: QrDensity = .init()
    @State private var frames: QrExportFrames? = nil
    @State private var error: String? = nil
    @State private var currentIndex = 0
    @State private var startedAt = Date()

    /// Whether to show the format picker (only if UR is available)
    var showFormatPicker: Bool {
        generateUr != nil
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
            QrExportHeader(title: title, canCopy: copyData != nil, copy: copyToClipboard)

            Text(subtitle)
                .font(.callout)
                .uiTestTruncationProbe()
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .lineLimit(nil)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 1)
                .padding(.horizontal, 40)

            if showFormatPicker {
                QrExportFormatPicker(selectedFormat: $selectedFormat)
            }

            QrExportContent(
                error: error,
                frames: frames,
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
            let frames: QrExportFrames = switch selectedFormat {
            case .bbqr:
                try await .loop(generateBbqrStrings(density))
            case .ur:
                if let generateUr {
                    try await .fountain(generateUr(density))
                } else {
                    // fallback to BBQr if UR not available
                    try await .loop(generateBbqrStrings(density))
                }
            }

            // restart the animation, so a UR shows its plain fragments in order before any mixed part
            self.frames = frames
            currentIndex = 0
            startedAt = Date()
            error = nil
        } catch let err {
            error = err.localizedDescription
            frames = nil
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

/// The export's title, with its copy button beside it
///
/// A hidden copy of the button on the other side keeps the title centered, so at accessibility
/// sizes the title wraps short of the button instead of running under it. The title keeps the
/// height of its wrapped lines, since the QR code below takes every point the sheet offers
private struct QrExportHeader: View {
    let title: String
    let canCopy: Bool
    let copy: () async -> Void

    var body: some View {
        HStack {
            if canCopy {
                copyButton.hidden().accessibilityHidden(true)
            }

            Text(title)
                .font(.title3)
                .fontWeight(.semibold)
                .uiTestTruncationProbe()
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity)

            if canCopy {
                copyButton
            }
        }
        .padding(.top, 12)
    }

    private var copyButton: some View {
        Button {
            Task { await copy() }
        } label: {
            Image(systemName: "doc.on.doc")
                .font(.body)
                .foregroundStyle(.secondary)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Copy")
        .padding(.horizontal, 4)
    }
}

private struct QrExportFormatPicker: View {
    @Binding var selectedFormat: QrExportFormat

    var body: some View {
        Picker("Format", selection: $selectedFormat) {
            ForEach(QrExportFormat.allCases, id: \.self) { format in
                Text(String(describing: format)).tag(format)
            }
        }
        .pickerStyle(.segmented)
        .padding(.vertical, 8)
        .frame(maxWidth: 200)
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
            generateUr: { density in try details.psbtToUrWithDensity(density: density) },
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
