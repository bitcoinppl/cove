//
//  BitcoinShieldIcon.swift
//  Cove
//
//  Created by Praveen Perera on 11/20/24.
//
import SwiftUI

// SVG
struct BitcoinShieldIcon: View {
    // args size
    var width: CGFloat? = nil
    var height: CGFloat? = nil

    // args colors
    var color: Color? = nil
    var shieldColor: Color? = nil
    var bitcoinColor: Color? = nil

    var isResizable = false
    func resizable() -> Self { Self(isResizable: true) }

    // private
    private var _shieldColor: Color { shieldColor ?? color ?? bitcoinColor ?? .primary }
    private var _bitcoinColor: Color { bitcoinColor ?? color ?? shieldColor ?? .primary }

    // this is the base size of the icon, everything is scaled from this
    private let staticSize = CGSize(width: 103, height: 125)
    private let defaultWidth: CGFloat = 17.0

    private var size: CGSize {
        if let width, let height {
            return CGSize(width: width, height: height)
        }

        if let width {
            return CGSize(width: width, height: width * widthHeightRatio)
        }

        if let height {
            return CGSize(width: height * heightWidthRatio, height: height)
        }

        return CGSize(width: defaultWidth, height: defaultWidth * widthHeightRatio)
    }

    private var widthHeightRatio: CGFloat {
        staticSize.height / staticSize.width
    }

    private var heightWidthRatio: CGFloat {
        staticSize.width / staticSize.height
    }

    struct ShieldShape: Shape {
        func path(in _: CGRect) -> Path {
            Path { path in
                path.move(to: CGPoint(x: 51.625, y: 124.688))
                path.addCurve(
                    to: CGPoint(x: 47.5, y: 123.375),
                    control1: CGPoint(x: 50.5, y: 124.688),
                    control2: CGPoint(x: 48.875, y: 124.188)
                )
                path.addCurve(
                    to: CGPoint(x: 0.937, y: 74),
                    control1: CGPoint(x: 12.625, y: 102.938),
                    control2: CGPoint(x: 0.937, y: 95.5)
                )
                path.addLine(to: CGPoint(x: 0.937, y: 26.5))
                path.addCurve(
                    to: CGPoint(x: 9.125, y: 15.312),
                    control1: CGPoint(x: 0.937, y: 19.812),
                    control2: CGPoint(x: 3.375, y: 17.562)
                )
                path.addCurve(
                    to: CGPoint(x: 46.688, y: 1.376),
                    control1: CGPoint(x: 18, y: 11.813),
                    control2: CGPoint(x: 37.813, y: 4.25)
                )
                path.addCurve(
                    to: CGPoint(x: 51.625, y: 0.438),
                    control1: CGPoint(x: 48.312, y: 0.876),
                    control2: CGPoint(x: 49.938, y: 0.438)
                )
                path.addCurve(
                    to: CGPoint(x: 56.625, y: 1.376),
                    control1: CGPoint(x: 53.313, y: 0.438),
                    control2: CGPoint(x: 54.938, y: 0.814)
                )
                path.addCurve(
                    to: CGPoint(x: 94.125, y: 15.313),
                    control1: CGPoint(x: 65.5, y: 4.438),
                    control2: CGPoint(x: 85.25, y: 11.75)
                )
                path.addCurve(
                    to: CGPoint(x: 102.312, y: 26.5),
                    control1: CGPoint(x: 99.875, y: 17.625),
                    control2: CGPoint(x: 102.312, y: 19.813)
                )
                path.addLine(to: CGPoint(x: 102.312, y: 74))
                path.addCurve(
                    to: CGPoint(x: 55.812, y: 123.375),
                    control1: CGPoint(x: 102.312, y: 95.5),
                    control2: CGPoint(x: 90.812, y: 103.125)
                )
                path.addCurve(
                    to: CGPoint(x: 51.625, y: 124.688),
                    control1: CGPoint(x: 54.375, y: 124.188),
                    control2: CGPoint(x: 52.812, y: 124.688)
                )
                path.closeSubpath()

                path.move(to: CGPoint(x: 51.625, y: 116.125))
                path.addCurve(
                    to: CGPoint(x: 55.25, y: 114.688),
                    control1: CGPoint(x: 52.688, y: 116.125),
                    control2: CGPoint(x: 53.813, y: 115.625)
                )
                path.addCurve(
                    to: CGPoint(x: 94.688, y: 72.5),
                    control1: CGPoint(x: 84.188, y: 96.562),
                    control2: CGPoint(x: 94.688, y: 91.5)
                )
                path.addLine(to: CGPoint(x: 94.688, y: 27.937))
                path.addCurve(
                    to: CGPoint(x: 91.75, y: 22.75),
                    control1: CGPoint(x: 94.688, y: 24.812),
                    control2: CGPoint(x: 94.125, y: 23.625)
                )
                path.addCurve(
                    to: CGPoint(x: 54.625, y: 8.875),
                    control1: CGPoint(x: 83.312, y: 19.687),
                    control2: CGPoint(x: 62.937, y: 12.125)
                )
                path.addCurve(
                    to: CGPoint(x: 51.625, y: 8.125),
                    control1: CGPoint(x: 53.437, y: 8.437),
                    control2: CGPoint(x: 52.375, y: 8.125)
                )
                path.addCurve(
                    to: CGPoint(x: 48.625, y: 8.875),
                    control1: CGPoint(x: 50.875, y: 8.125),
                    control2: CGPoint(x: 49.812, y: 8.375)
                )
                path.addCurve(
                    to: CGPoint(x: 11.5, y: 22.75),
                    control1: CGPoint(x: 40.312, y: 12.125),
                    control2: CGPoint(x: 19.875, y: 19.438)
                )
                path.addCurve(
                    to: CGPoint(x: 8.562, y: 27.938),
                    control1: CGPoint(x: 9.187, y: 23.688),
                    control2: CGPoint(x: 8.562, y: 24.813)
                )
                path.addLine(to: CGPoint(x: 8.562, y: 72.5))
                path.addCurve(
                    to: CGPoint(x: 48, y: 114.688),
                    control1: CGPoint(x: 8.563, y: 91.5),
                    control2: CGPoint(x: 19, y: 96.688)
                )
                path.addCurve(
                    to: CGPoint(x: 51.625, y: 116.125),
                    control1: CGPoint(x: 49.438, y: 115.625),
                    control2: CGPoint(x: 50.625, y: 116.125)
                )
                path.closeSubpath()
            }
        }
    }

    struct BitcoinShape: Shape {
        func path(in _: CGRect) -> Path {
            Path { path in
                path.move(to: CGPoint(x: 40.22, y: 85.453))
                path.addCurve(
                    to: CGPoint(x: 36.47, y: 81.703),
                    control1: CGPoint(x: 37.818, y: 85.453),
                    control2: CGPoint(x: 36.47, y: 83.813)
                )
                path.addLine(to: CGPoint(x: 36.47, y: 37.728))
                path.addCurve(
                    to: CGPoint(x: 40.22, y: 33.978),
                    control1: CGPoint(x: 36.47, y: 35.502),
                    control2: CGPoint(x: 37.965, y: 33.978)
                )
                path.addLine(to: CGPoint(x: 43.62, y: 33.978))
                path.addLine(to: CGPoint(x: 43.62, y: 28.324))
                path.addCurve(
                    to: CGPoint(x: 45.553, y: 26.42),
                    control1: CGPoint(x: 43.62, y: 27.211),
                    control2: CGPoint(x: 44.41, y: 26.42)
                )
                path.addCurve(
                    to: CGPoint(x: 47.486, y: 28.324),
                    control1: CGPoint(x: 46.695, y: 26.42),
                    control2: CGPoint(x: 47.486, y: 27.21)
                )
                path.addLine(to: CGPoint(x: 47.486, y: 33.978))
                path.addLine(to: CGPoint(x: 52.994, y: 33.978))
                path.addLine(to: CGPoint(x: 52.994, y: 28.324))
                path.addCurve(
                    to: CGPoint(x: 54.957, y: 26.42),
                    control1: CGPoint(x: 52.994, y: 27.211),
                    control2: CGPoint(x: 53.814, y: 26.42)
                )
                path.addCurve(
                    to: CGPoint(x: 56.832, y: 28.324),
                    control1: CGPoint(x: 56.041, y: 26.42),
                    control2: CGPoint(x: 56.832, y: 27.21)
                )
                path.addLine(to: CGPoint(x: 56.832, y: 34.096))
                path.addCurve(
                    to: CGPoint(x: 69.693, y: 46.723),
                    control1: CGPoint(x: 64.244, y: 34.886),
                    control2: CGPoint(x: 69.693, y: 39.369)
                )
                path.addCurve(
                    to: CGPoint(x: 60.231, y: 58.148),
                    control1: CGPoint(x: 69.693, y: 52.26),
                    control2: CGPoint(x: 65.709, y: 57.182)
                )
                path.addLine(to: CGPoint(x: 60.231, y: 58.471))
                path.addCurve(
                    to: CGPoint(x: 72.359, y: 71.186),
                    control1: CGPoint(x: 67.496, y: 59.321),
                    control2: CGPoint(x: 72.359, y: 64.271)
                )
                path.addCurve(
                    to: CGPoint(x: 56.832, y: 85.366),
                    control1: CGPoint(x: 72.359, y: 80.268),
                    control2: CGPoint(x: 65.621, y: 84.721)
                )
                path.addLine(to: CGPoint(x: 56.832, y: 91.313))
                path.addCurve(
                    to: CGPoint(x: 54.957, y: 93.246),
                    control1: CGPoint(x: 56.832, y: 92.426),
                    control2: CGPoint(x: 56.041, y: 93.246)
                )
                path.addCurve(
                    to: CGPoint(x: 52.994, y: 91.313),
                    control1: CGPoint(x: 53.815, y: 93.246),
                    control2: CGPoint(x: 52.994, y: 92.426)
                )
                path.addLine(to: CGPoint(x: 52.994, y: 85.453))
                path.addLine(to: CGPoint(x: 47.486, y: 85.453))
                path.addLine(to: CGPoint(x: 47.486, y: 91.313))
                path.addCurve(
                    to: CGPoint(x: 45.553, y: 93.246),
                    control1: CGPoint(x: 47.486, y: 92.426),
                    control2: CGPoint(x: 46.696, y: 93.246)
                )
                path.addCurve(
                    to: CGPoint(x: 43.619, y: 91.313),
                    control1: CGPoint(x: 44.41, y: 93.246),
                    control2: CGPoint(x: 43.619, y: 92.426)
                )
                path.addLine(to: CGPoint(x: 43.619, y: 85.453))
                path.addLine(to: CGPoint(x: 40.221, y: 85.453))
                path.closeSubpath()

                path.move(to: CGPoint(x: 42.273, y: 56.391))
                path.addLine(to: CGPoint(x: 51.5, y: 56.391))
                path.addCurve(
                    to: CGPoint(x: 63.834, y: 47.367),
                    control1: CGPoint(x: 58.121, y: 56.391),
                    control2: CGPoint(x: 63.834, y: 53.988)
                )
                path.addCurve(
                    to: CGPoint(x: 53.141, y: 39.047),
                    control1: CGPoint(x: 63.834, y: 41.537),
                    control2: CGPoint(x: 59.117, y: 39.047)
                )
                path.addLine(to: CGPoint(x: 42.27, y: 39.047))
                path.addLine(to: CGPoint(x: 42.27, y: 56.39))
                path.closeSubpath()

                path.move(to: CGPoint(x: 42.27, y: 80.384))
                path.addLine(to: CGPoint(x: 53.607, y: 80.384))
                path.addCurve(
                    to: CGPoint(x: 66.439, y: 70.892),
                    control1: CGPoint(x: 60.667, y: 80.384),
                    control2: CGPoint(x: 66.439, y: 77.864)
                )
                path.addCurve(
                    to: CGPoint(x: 52.933, y: 61.429),
                    control1: CGPoint(x: 66.439, y: 63.714),
                    control2: CGPoint(x: 60.169, y: 61.429)
                )
                path.addLine(to: CGPoint(x: 42.272, y: 61.429))
                path.addLine(to: CGPoint(x: 42.272, y: 80.384))
                path.closeSubpath()
            }
        }
    }

    @ViewBuilder
    var icon: some View {
        ShieldShape().fill(_shieldColor)
        BitcoinShape().fill(_bitcoinColor)
    }

    var body: some View {
        if isResizable {
            GeometryReader { proxy in
                ZStack { icon }
                    .frame(width: size.width, height: size.height)
                    .scaleEffect(
                        x: proxy.size.width / size.width,
                        y: proxy.size.height / size.height
                    )
                    .frame(width: proxy.size.width, height: proxy.size.height)
            }
        } else {
            ZStack { icon }
                .frame(width: 103, height: 125)
                .scaleEffect(
                    x: size.width / 103,
                    y: size.height / 125
                )
                .frame(width: size.width, height: size.height)
        }
    }
}

#Preview("default") {
    HStack {
        BitcoinShieldIcon()
        BitcoinShieldIcon(width: 50)
        BitcoinShieldIcon(height: 125)
    }
}

#Preview("colors") {
    HStack {
        BitcoinShieldIcon(width: 100, color: .orange)
        BitcoinShieldIcon(width: 100, shieldColor: .orange, bitcoinColor: .orange.opacity(0.5))
        BitcoinShieldIcon(width: 100, bitcoinColor: .red)
    }
}
