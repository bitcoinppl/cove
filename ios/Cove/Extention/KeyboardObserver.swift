import Foundation
import SwiftUI
import UIKit

@Observable
@MainActor
final class KeyboardObserver {
    var keyboardIsShowing = false

    // nonisolated so they can be accessed in deinit
    private nonisolated(unsafe) var showObserver: NSObjectProtocol?
    private nonisolated(unsafe) var hideObserver: NSObjectProtocol?

    init() {
        showObserver = NotificationCenter.default.addObserver(
            forName: UIResponder.keyboardWillShowNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            guard let self else { return }

            Task { @MainActor in
                withAnimation(.easeInOut(duration: 0.25)) {
                    self.keyboardIsShowing = true
                }
            }
        }

        hideObserver = NotificationCenter.default.addObserver(
            forName: UIResponder.keyboardWillHideNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            guard let self else { return }

            Task { @MainActor in
                withAnimation(.easeInOut(duration: 0.25)) {
                    self.keyboardIsShowing = false
                }
            }
        }
    }

    deinit {
        if let showObserver { NotificationCenter.default.removeObserver(showObserver) }
        if let hideObserver { NotificationCenter.default.removeObserver(hideObserver) }
    }
}
