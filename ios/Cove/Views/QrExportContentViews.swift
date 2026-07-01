import SwiftUI

struct QrExportContent: View {
    let error: String?
    let qrs: [QrCodeView]
    @Binding var currentIndex: Int
    @Binding var density: QrDensity
    let startedAt: Date
    let animationInterval: TimeInterval

    var body: some View {
        if let error {
            Text(verbatim: error)
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
