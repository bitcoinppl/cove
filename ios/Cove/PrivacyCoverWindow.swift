//
//  PrivacyCoverWindow.swift
//  Cove
//
//  The privacy cover, in a window of its own above the app's
//

import SwiftUI
import UIKit

extension View {
    /// Covers the scene this view is in with `CoverView` while `isPresented` is true
    ///
    /// The cover sits in a window of its own, above the app's windows and its alerts, so it shows at
    /// once whatever the app is presenting, and fades out when cleared. As a `.fullScreenCover` it
    /// shared the app's presentation chain: SwiftUI held it back while a sheet a screen presented,
    /// such as Receive, was up, which left the sheet in the app switcher, and it could stay up after
    /// the app came back, which the app worked around by clearing it again on a timer
    func privacyCover(isPresented: Bool) -> some View {
        background(PrivacyCoverAnchor(isPresented: isPresented))
    }
}

private struct PrivacyCoverAnchor: UIViewRepresentable {
    let isPresented: Bool

    func makeUIView(context _: Context) -> PrivacyCoverAnchorView {
        PrivacyCoverAnchorView()
    }

    func updateUIView(_ view: PrivacyCoverAnchorView, context _: Context) {
        view.isCoverPresented = isPresented
    }
}

/// Shows the privacy cover's window in the scene this view is in, while the cover is presented
///
/// The window is built each time the cover shows and released once it has faded out, so the cover's
/// view starts and stops with each showing, as it did when it was presented
private final class PrivacyCoverAnchorView: UIView {
    private static let fadeOut: TimeInterval = 0.2

    var isCoverPresented = false {
        didSet {
            guard oldValue != isCoverPresented else { return }
            update()
        }
    }

    private var coverWindow: UIWindow?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        accessibilityElementsHidden = true
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()

        // a cover belongs to the scene it was built in, so moving drops it and builds one for the
        // new scene, if any
        tearDownCover()
        update()
    }

    private func update() {
        guard let scene = window?.windowScene else { return }

        if isCoverPresented {
            showCover(in: scene)
        } else {
            hideCover()
        }
    }

    private func showCover(in scene: UIWindowScene) {
        // a cover still fading out comes straight back
        if let coverWindow {
            coverWindow.layer.removeAllAnimations()
            coverWindow.alpha = 1
            return
        }

        let cover = UIWindow(windowScene: scene)

        // above the app's alerts, so nothing the app presents is left uncovered
        cover.windowLevel = .alert + 1
        cover.rootViewController = UIHostingController(rootView: PrivacyCoverContent())
        cover.isHidden = false
        coverWindow = cover
    }

    private func hideCover() {
        guard let cover = coverWindow else { return }

        UIView.animate(withDuration: Self.fadeOut) {
            cover.alpha = 0
        } completion: { [weak self] _ in
            // a cover shown again while it faded stays up
            guard let self, !isCoverPresented, coverWindow === cover else { return }
            tearDownCover()
        }
    }

    private func tearDownCover() {
        coverWindow?.isHidden = true
        coverWindow = nil
    }
}

private struct PrivacyCoverContent: View {
    var body: some View {
        CoverView()
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("privacy.cover")
    }
}
