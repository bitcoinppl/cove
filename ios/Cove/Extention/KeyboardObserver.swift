import Foundation
import SwiftUI
import UIKit

class KeyboardObserver: ObservableObject {
    @Published var keyboardIsShowing = false

    init() {
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardWillShow), name: UIResponder.keyboardWillShowNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardWillHide), name: UIResponder.keyboardWillHideNotification, object: nil)
    }

    @objc func keyboardWillShow() {
        withAnimation(.easeInOut(duration: 0.25)) {
            keyboardIsShowing = true
        }
    }

    @objc func keyboardWillHide() {
        withAnimation(.easeInOut(duration: 0.25)) {
            keyboardIsShowing = false
        }
    }
}
