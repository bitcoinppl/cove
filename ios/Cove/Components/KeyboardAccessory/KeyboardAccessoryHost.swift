//
//  KeyboardAccessoryHost.swift
//  Cove
//
//  Created by ChatGPT on 11/19/25.
//

import SwiftUI
import UIKit

/// Bridges a SwiftUI accessory view into the native `inputAccessoryView` of the current first responder.
struct KeyboardAccessoryHost<Accessory: View>: UIViewRepresentable {
    var controller: KeyboardAccessoryController
    var isVisible: Bool = true
    var height: CGFloat
    @ViewBuilder var accessory: () -> Accessory

    func makeUIView(context _: Context) -> UIView {
        UIView(frame: .zero)
    }

    func updateUIView(_ uiView: UIView, context _: Context) {
        // Capture the current first responder each pass.
        UIResponder.captureCurrentFirstResponder(from: uiView.window)
        controller.update(isVisible: isVisible, height: height) {
            AnyView(accessory())
        }
    }
}

// MARK: - Controller

final class KeyboardAccessoryController: ObservableObject {
    private var hosting: UIHostingController<AnyView>?
    private var container: UIView?
    private weak var currentResponder: UIView?
    private var isAttached: Bool = false

    func update(isVisible: Bool, height: CGFloat, @ViewBuilder accessory: () -> AnyView) {
        guard let responderView = UIResponder.currentFirstResponderView else {
            return
        }

        // check if visibility or responder changed
        let responderChanged = currentResponder !== responderView
        let needsAttachment = isVisible && (!isAttached || responderChanged)
        let needsDetachment = !isVisible && isAttached

        // remove when hidden; keeps native keyboard height stable
        if needsDetachment {
            setAccessory(on: responderView, accessoryView: nil, forceReload: true)
            currentResponder = nil
            isAttached = false
            return
        }

        let rootView = accessory()
        let hosting = hosting ?? UIHostingController(rootView: rootView)
        hosting.rootView = rootView
        hosting.view.backgroundColor = .clear
        hosting.view.translatesAutoresizingMaskIntoConstraints = false
        hosting.view.isUserInteractionEnabled = true

        let container = container ?? UIView(frame: CGRect(x: 0, y: 0, width: UIScreen.main.bounds.width, height: height))
        container.backgroundColor = .clear
        container.autoresizingMask = [.flexibleWidth]
        container.isUserInteractionEnabled = true

        if hosting.view.superview != container {
            container.subviews.forEach { $0.removeFromSuperview() }
            container.addSubview(hosting.view)
            NSLayoutConstraint.activate([
                hosting.view.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                hosting.view.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                hosting.view.topAnchor.constraint(equalTo: container.topAnchor),
                hosting.view.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            ])
        }

        self.hosting = hosting
        self.container = container

        // only set the accessory view when actually visible
        if isVisible {
            // only reload when responder changes, not on first attachment (for smooth animation)
            setAccessory(on: responderView, accessoryView: container, forceReload: responderChanged)
            currentResponder = responderView
            isAttached = true
        }
    }

    private func setAccessory(on responder: UIView, accessoryView: UIView?, forceReload: Bool) {
        if let textField = responder as? UITextField {
            textField.inputAccessoryView = accessoryView
            if forceReload {
                textField.reloadInputViews()
            }
        } else if let textView = responder as? UITextView {
            textView.inputAccessoryView = accessoryView
            if forceReload {
                textView.reloadInputViews()
            }
        } else if let searchBar = responder as? UISearchBar {
            searchBar.inputAccessoryView = accessoryView
            if forceReload {
                searchBar.reloadInputViews()
            }
        }
    }
}

// MARK: - First responder helper

extension UIResponder {
    private weak static var currentResponder: UIResponder?

    static var currentFirstResponderView: UIView? {
        currentResponder as? UIView
    }

    @discardableResult
    static func captureCurrentFirstResponder(from _: UIWindow?) -> UIResponder? {
        currentResponder = nil
        UIApplication.shared.sendAction(#selector(findFirstResponder(_:)), to: nil, from: nil, for: nil)
        return currentResponder
    }

    @objc private func findFirstResponder(_: Any) {
        UIResponder.currentResponder = self
    }
}
