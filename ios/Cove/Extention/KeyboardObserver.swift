import Foundation
import SwiftUI
import UIKit

@MainActor
class KeyboardObserver: ObservableObject {
    @Published var keyboardIsShowing = false

    init() {
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardWillShow), name: UIResponder.keyboardWillShowNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardWillHide), name: UIResponder.keyboardWillHideNotification, object: nil)
    }

    @objc func keyboardWillShow() {
        Task { @MainActor in
            withAnimation(.easeInOut(duration: 0.25)) {
                self.keyboardIsShowing = true
            }
        }
    }

    @objc func keyboardWillHide() {
        Task { @MainActor in
            withAnimation(.easeInOut(duration: 0.25)) {
                self.keyboardIsShowing = false
            }
        }
    }
}
