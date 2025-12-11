import Foundation
import SwiftUI
import UIKit

@Observable
@MainActor
final class KeyboardObserver {
    var keyboardIsShowing = false
    var keyboardHeight: CGFloat = 0

    // nonisolated so they can be accessed in deinit
    private nonisolated(unsafe) var showObserver: NSObjectProtocol?
    private nonisolated(unsafe) var hideObserver: NSObjectProtocol?

    init() {
        showObserver = NotificationCenter.default.addObserver(
            forName: UIResponder.keyboardWillShowNotification,
            object: nil,
            queue: .main
        ) { [weak self] notification in
            guard let self else { return }
            let keyboardFrame = notification.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect

            Task { @MainActor in
                withAnimation(.easeInOut(duration: 0.25)) {
                    self.keyboardIsShowing = true
                    self.keyboardHeight = keyboardFrame?.height ?? 0
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
                    self.keyboardHeight = 0
                }
            }
        }
    }

    deinit {
        if let showObserver { NotificationCenter.default.removeObserver(showObserver) }
        if let hideObserver { NotificationCenter.default.removeObserver(hideObserver) }
    }
}
