import SwiftUI
import UIKit

/// The background a QR sheet draws behind its code
enum QrSheetBackground {
    /// Light on opaque white whatever the app's appearance, for a code drawn straight onto the
    /// sheet, since cheap scanner cameras, such as SeedSigner's, fail to find one on a dark sheet
    case alwaysLight
    /// The app's own appearance, for a sheet whose code sits on a `QrCodeCard`, whose white quiet
    /// zone lets a camera find the code on a dark sheet
    case appAppearance
}

extension View {
    /// Presents a sheet that shows a QR code for another device to scan
    ///
    /// The screen goes to full brightness while the sheet is shown and returns to where the user
    /// had it once it closes. `background` sets whether the sheet stays light in dark mode
    ///
    /// Apply it to the sheet's root view
    func qrSheetPresentation(_ background: QrSheetBackground = .alwaysLight) -> some View {
        modifier(QrSheetPresentation(background: background))
    }
}

private struct QrSheetPresentation: ViewModifier {
    let background: QrSheetBackground
    @Environment(\.scenePhase) private var scenePhase

    func body(content: Content) -> some View {
        styled(content)
            .onAppear { QrScreenBrightness.shared.hold() }
            .onDisappear { QrScreenBrightness.shared.release() }
            .onChange(of: scenePhase) { _, phase in
                QrScreenBrightness.shared.sceneChanged(to: phase)
            }
    }

    @ViewBuilder
    private func styled(_ content: Content) -> some View {
        switch background {
        case .alwaysLight:
            content
                .preferredColorScheme(.light)
                // opaque, since a sheet below full height otherwise draws on translucent glass,
                // which shows the dark screen behind it as grey around the QR code
                .presentationBackground(.white)

        case .appAppearance:
            content
        }
    }
}

/// Keeps the screen at full brightness while any QR sheet holds it
///
/// Screen brightness belongs to the device, not the app, and stays where an app set it after the
/// app leaves the foreground. So it goes back to the user's level whenever the app is inactive and
/// is raised again when the app returns with a QR sheet still open. Holds are counted, since a
/// sheet replacing another appears before the old one disappears
@MainActor
private final class QrScreenBrightness {
    static let shared = QrScreenBrightness()

    private var holders = 0
    /// The user's brightness, kept while the screen is raised
    private var userBrightness: CGFloat?

    func hold() {
        holders += 1
        raise()
    }

    func release() {
        holders = max(0, holders - 1)
        if holders == 0 { restore() }
    }

    func sceneChanged(to phase: ScenePhase) {
        guard phase == .active else { return restore() }
        if holders > 0 { raise() }
    }

    private func raise() {
        guard userBrightness == nil, let screen else { return }

        userBrightness = screen.brightness
        screen.brightness = 1
    }

    private func restore() {
        guard let brightness = userBrightness, let screen else { return }

        userBrightness = nil
        screen.brightness = brightness
    }

    private var screen: UIScreen? {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first?.screen
    }
}
