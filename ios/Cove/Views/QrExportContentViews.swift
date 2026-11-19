import SwiftUI

struct QrExportContent: View {
    let error: String?
    let frames: QrExportFrames?
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
        } else if let frames, frames.passCount > 0 {
            QrExportAnimatedQrView(
                frames: frames,
                currentIndex: $currentIndex,
                density: $density,
                startedAt: startedAt,
                animationInterval: animationInterval
            )
        } else {
            ProgressView()
                .padding(.top, 20)
        }
    }
}

private struct QrExportAnimatedQrView: View {
    let frames: QrExportFrames
    @Binding var currentIndex: Int
    @Binding var density: QrDensity
    let startedAt: Date
    let animationInterval: TimeInterval

    var body: some View {
        VStack {
            // .id() forces TimelineView recreation when interval changes
            TimelineView(.periodic(from: startedAt, by: animationInterval)) { context in
                let tick = abs(Int(context.date.distance(to: startedAt) / animationInterval))
                QrCodeView(text: frames.frame(at: tick))
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 11)
                    .onChange(of: tick) { _, newTick in
                        // the counter counts a pass of the plain fragments, and starts over as a
                        // UR's mixed parts go on past them
                        currentIndex = newTick % frames.passCount
                    }
            }
            .id(animationInterval)

            if frames.passCount > 1 {
                HStack(alignment: .center, spacing: 8) {
                    QrExportDensityButton(
                        systemImage: "minus",
                        label: "Decrease density",
                        size: 44,
                        isEnabled: canDecreaseDensity,
                        action: decreaseDensity
                    )

                    QrExportProgressIndicator(qrCount: frames.passCount, currentIndex: currentIndex)

                    QrExportDensityButton(
                        systemImage: "plus",
                        label: "Increase density",
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
        density.canIncrease() && frames.passCount > 1
    }

    private func decreaseDensity() {
        density = density.decrease()
    }

    private func increaseDensity() {
        density = density.increase()
    }
}

struct QrExportProgressIndicator: View {
    let qrCount: Int
    let currentIndex: Int

    var body: some View {
        Canvas { context, size in
            let minimumSegmentWidth: CGFloat = 1
            let preferredSpacing: CGFloat = 4
            let spacing = min(
                preferredSpacing,
                max(0, (size.width - (CGFloat(qrCount) * minimumSegmentWidth)) / CGFloat(qrCount - 1))
            )
            let segmentWidth = max(
                0,
                (size.width - (CGFloat(qrCount - 1) * spacing)) / CGFloat(qrCount)
            )
            let cornerRadius = min(2, segmentWidth / 2, size.height / 2)

            for index in 0 ..< qrCount {
                let rect = CGRect(
                    x: CGFloat(index) * (segmentWidth + spacing),
                    y: 0,
                    width: segmentWidth,
                    height: size.height
                )
                let color = Color.blue.opacity(index == currentIndex ? 1 : 0.3)

                context.fill(
                    Path(roundedRect: rect, cornerRadius: cornerRadius),
                    with: .color(color)
                )
            }
        }
        .frame(maxWidth: .infinity)
        .frame(height: 12)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("QR frame")
        .accessibilityValue("\(currentIndex + 1) of \(qrCount)")
    }
}

private struct QrExportDensityButton: View {
    let systemImage: String
    /// The same name Android gives the button, so both platforms read and find it alike
    let label: LocalizedStringKey
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
        .accessibilityLabel(label)
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
                label: "Decrease density",
                size: 32,
                isEnabled: canDecrease,
                action: decrease
            )

            Divider()
                .frame(height: 20)

            QrExportDensityButton(
                systemImage: "plus",
                label: "Increase density",
                size: 32,
                isEnabled: canIncrease,
                action: increase
            )
        }
        .background(Color.secondary.opacity(0.15))
        .cornerRadius(50)
    }
}
