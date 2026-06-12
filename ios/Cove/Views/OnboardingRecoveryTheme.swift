import SwiftUI

enum OnboardingRecoveryTypography {
    static let termsTitle = Font.largeTitle.weight(.bold)
    static let heroTitle = Font.title.weight(.bold)
    static let compactTitle = Font.title2.weight(.semibold)
    static let body = Font.body
    static let bodySemibold = Font.body.weight(.semibold)
    static let subheadline = Font.subheadline
    static let footnote = Font.footnote
    static let captionSemibold = Font.caption.weight(.semibold)
    static let primaryButton = Font.headline.weight(.semibold)
    static let secondaryButton = Font.body.weight(.semibold)
}

struct OnboardingStepIndicator: View {
    let selected: Int
    var total: Int = 3

    var body: some View {
        HStack(spacing: 9) {
            ForEach(0 ..< total, id: \.self) { index in
                if index == selected {
                    Capsule()
                        .fill(.white)
                        .frame(width: 24, height: 6)
                } else {
                    Circle()
                        .fill(.white.opacity(0.22))
                        .frame(width: 6, height: 6)
                }
            }
        }
        .frame(maxWidth: .infinity)
    }
}

struct OnboardingStatusHero: View {
    let systemImage: String
    var tint: Color = .btnGradientLight
    var fillColor: Color = .duskBlue.opacity(0.42)
    var pulse = false
    var iconSize: CGFloat = 24
    var ringSizes: [CGFloat] = [118, 86, 58]

    var body: some View {
        TimelineView(.animation) { context in
            let ringScale = pulse ? ringScale(at: context.date) : 1

            ZStack {
                ForEach(Array(ringSizes.enumerated()), id: \.offset) { index, size in
                    Circle()
                        .stroke(tint.opacity(ringOpacity(for: index)), lineWidth: 1)
                        .frame(width: size, height: size)
                        .scaleEffect(ringScale)
                }

                Circle()
                    .fill(fillColor)
                    .frame(width: 58, height: 58)

                Circle()
                    .stroke(tint.opacity(pulse ? 0.88 : 0.7), lineWidth: 1.3)
                    .frame(width: 58, height: 58)

                Image(systemName: systemImage)
                    .font(.system(size: iconSize, weight: .semibold))
                    .foregroundStyle(tint)
            }
        }
        .frame(width: 118, height: 118)
    }

    private func ringOpacity(for index: Int) -> Double {
        switch index {
        case 0: 0.15
        case 1: 0.22
        default: 0.34
        }
    }

    private func ringScale(at date: Date) -> CGFloat {
        let duration = 1.85
        let phase = date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: duration * 2)
        let progress = phase <= duration ? phase / duration : 2 - phase / duration

        return 0.96 + (0.10 * CGFloat(progress))
    }
}

struct OnboardingThinProgressBar: View {
    let progress: Double

    var body: some View {
        GeometryReader { geometry in
            let clampedProgress = min(max(progress, 0), 1)
            let fillWidth = geometry.size.width * clampedProgress

            ZStack(alignment: .leading) {
                Capsule()
                    .fill(Color.white.opacity(0.12))

                Capsule()
                    .fill(Color.btnGradientLight)
                    .frame(width: fillWidth)
            }
        }
        .frame(width: 164, height: 5)
        .animation(.easeInOut(duration: 0.25), value: progress)
    }
}

struct OnboardingPrimaryButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(OnboardingRecoveryTypography.primaryButton)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 18)
            .padding(.horizontal, 18)
            .foregroundStyle(.white.opacity(isEnabled ? 1 : 0.45))
            .background(
                LinearGradient(
                    colors: [.btnGradientLight, .btnGradientDark],
                    startPoint: .leading,
                    endPoint: .trailing
                ),
                in: RoundedRectangle(cornerRadius: 16, style: .continuous)
            )
            .opacity(isEnabled ? (configuration.isPressed ? 0.84 : 1) : 0.45)
    }
}

struct OnboardingSecondaryButtonStyle: ButtonStyle {
    var backgroundColor: Color = .duskBlue.opacity(0.58)
    var foregroundColor: Color = .white
    var borderColor: Color = .coveLightGray.opacity(0.12)

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(OnboardingRecoveryTypography.secondaryButton)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 17)
            .padding(.horizontal, 18)
            .foregroundStyle(foregroundColor)
            .background(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill(backgroundColor)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(borderColor, lineWidth: 1)
            )
            .opacity(configuration.isPressed ? 0.84 : 1)
    }
}

private struct OnboardingRecoveryBackgroundModifier: ViewModifier {
    func body(content: Content) -> some View {
        content.background {
            ZStack {
                Color.midnightBlue

                RadialGradient(
                    stops: [
                        .init(color: Color(red: 0.165, green: 0.353, blue: 0.545).opacity(0.92), location: 0),
                        .init(color: Color(red: 0.118, green: 0.227, blue: 0.361).opacity(0.45), location: 0.4),
                        .init(color: .clear, location: 0.84),
                    ],
                    center: .init(x: 0.33, y: 0.16),
                    startRadius: 0,
                    endRadius: 420
                )

                RadialGradient(
                    stops: [
                        .init(color: Color(red: 0.118, green: 0.290, blue: 0.420).opacity(0.82), location: 0),
                        .init(color: .clear, location: 0.74),
                    ],
                    center: .init(x: 0.78, y: 0.1),
                    startRadius: 0,
                    endRadius: 320
                )
            }
            .ignoresSafeArea()
        }
    }
}

extension View {
    func onboardingRecoveryBackground() -> some View {
        modifier(OnboardingRecoveryBackgroundModifier())
    }
}
