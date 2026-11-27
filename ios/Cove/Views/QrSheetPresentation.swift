import SwiftUI
import UIKit

extension View {
    /// Presents a sheet that shows a QR code for another device to scan
    ///
    /// The sheet stays in light mode on an opaque white background whatever the app's appearance,
    /// since cheap scanner cameras, such as SeedSigner's, fail to find a QR code drawn on a dark
    /// sheet. The screen goes to full brightness while the sheet is shown and returns to where the
    /// user had it once it closes
    ///
    /// Apply it to the sheet's root view
    func qrSheetPresentation() -> some View {
        modifier(QrSheetPresentation())
    }
}

private struct QrSheetPresentation: ViewModifier {
    @Environment(\.scenePhase) private var scenePhase

    func body(content: Content) -> some View {
        content
            .preferredColorScheme(.light)
            // opaque, since a sheet below full height otherwise draws on translucent glass, which
            // shows the dark screen behind it as grey around the QR code
            .presentationBackground(.white)
            .onAppear { QrScreenBrightness.shared.hold() }
            .onDisappear { QrScreenBrightness.shared.release() }
            .onChange(of: scenePhase) { _, phase in
                QrScreenBrightness.shared.sceneChanged(to: phase)
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
