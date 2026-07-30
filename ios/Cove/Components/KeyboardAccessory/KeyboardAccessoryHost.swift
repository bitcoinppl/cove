//
//  KeyboardAccessoryHost.swift
//  Cove
//
//  Created by Praveen Perera on 11/19/25.
//

import SwiftUI
import UIKit

/// Bridges a SwiftUI accessory view into the native `inputAccessoryView` of the current first responder.
struct KeyboardAccessoryHost<Accessory: View>: UIViewRepresentable {
    var isVisible: Bool = true
    var height: CGFloat
    @ViewBuilder var accessory: () -> Accessory

    func makeCoordinator() -> KeyboardAccessoryController {
        KeyboardAccessoryController()
    }

    func makeUIView(context _: Context) -> UIView {
        UIView(frame: .zero)
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        // Capture the current first responder each pass.
        UIResponder.captureCurrentFirstResponder(from: uiView.window)
        context.coordinator.update(isVisible: isVisible, height: height) {
            AnyView(accessory())
        }
    }

    static func dismantleUIView(_: UIView, coordinator: KeyboardAccessoryController) {
        coordinator.detach()
    }
}

// MARK: - Controller

final class KeyboardAccessoryController {
    private var hosting: UIHostingController<AnyView>?
    private var container: UIView?
    private weak var currentResponder: UIView?
    private var shouldShowAccessory: Bool = false
    private var didBecomeActiveObserver: NSObjectProtocol?
    private var keyboardDidShowObserver: NSObjectProtocol?

    init() {
        didBecomeActiveObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.didBecomeActiveNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.reattachIfNeeded()
        }

        keyboardDidShowObserver = NotificationCenter.default.addObserver(
            forName: UIResponder.keyboardWillShowNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.reattachOnKeyboardShow()
        }
    }

    /// Reattach the accessory view when app returns from background/notification center.
    /// This handles the case where iOS clears the inputAccessoryView but SwiftUI doesn't trigger an update.
    private func reattachIfNeeded() {
        // small delay to let iOS complete its transition
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self] in
            guard let self,
                  self.shouldShowAccessory,
                  let container = self.container
            else {
                return
            }

            UIResponder.captureCurrentFirstResponder(from: nil)
            guard let responder = UIResponder.currentFirstResponderView else { return }

            self.detachFromPreviousResponder(unless: responder)

            // force rebuild: clear first, then re-set
            self.setAccessory(on: responder, accessoryView: nil, forceReload: false)
            self.setAccessory(on: responder, accessoryView: container, forceReload: true)
            self.currentResponder = responder
        }
    }

    /// Reattach accessory when keyboard appears after focus transitions.
    /// Handles the case where first responder changes between text fields.
    private func reattachOnKeyboardShow() {
        guard shouldShowAccessory, let container else { return }

        // re-capture first responder
        UIResponder.captureCurrentFirstResponder(from: nil)
        guard let responder = UIResponder.currentFirstResponderView else { return }

        detachFromPreviousResponder(unless: responder)

        guard accessoryView(on: responder) !== container else {
            currentResponder = responder
            return
        }

        // record attachment before reloadInputViews can emit another keyboard notification
        currentResponder = responder

        // force rebuild: clear first, then re-set
        self.setAccessory(on: responder, accessoryView: nil, forceReload: false)
        self.setAccessory(on: responder, accessoryView: container, forceReload: true)
    }

    deinit {
        if let observer = didBecomeActiveObserver {
            NotificationCenter.default.removeObserver(observer)
        }
        if let observer = keyboardDidShowObserver {
            NotificationCenter.default.removeObserver(observer)
        }
    }

    func update(isVisible: Bool, height: CGFloat, @ViewBuilder accessory: () -> AnyView) {
        shouldShowAccessory = isVisible

        guard isVisible else {
            detach()
            return
        }

        guard let responderView = UIResponder.currentFirstResponderView else {
            return
        }

        let rootView = accessory()
        let hosting = hosting ?? UIHostingController(rootView: rootView)
        hosting.rootView = rootView
        hosting.view.backgroundColor = .clear
        hosting.view.translatesAutoresizingMaskIntoConstraints = false
        hosting.view.isUserInteractionEnabled = true

        let container =
            container
                ?? UIView(frame: CGRect(x: 0, y: 0, width: UIScreen.main.bounds.width, height: height))
        let heightChanged = container.frame.height != height
        container.frame.size.height = height
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

        let responderChanged = currentResponder !== responderView
        detachFromPreviousResponder(unless: responderView)

        if accessoryView(on: responderView) !== container || heightChanged {
            // reload only when an existing input view must change
            let forceReload = responderChanged || heightChanged
            setAccessory(on: responderView, accessoryView: container, forceReload: forceReload)
        }

        currentResponder = responderView
    }

    func detach() {
        shouldShowAccessory = false

        guard let responder = currentResponder else { return }

        if accessoryView(on: responder) === container {
            setAccessory(on: responder, accessoryView: nil, forceReload: responder.isFirstResponder)
        }

        currentResponder = nil
    }

    private func detachFromPreviousResponder(unless responder: UIView) {
        guard let currentResponder, currentResponder !== responder else { return }

        if accessoryView(on: currentResponder) === container {
            setAccessory(on: currentResponder, accessoryView: nil, forceReload: false)
        }

        self.currentResponder = nil
    }

    private func accessoryView(on responder: UIView) -> UIView? {
        if let textField = responder as? UITextField {
            return textField.inputAccessoryView
        }
        if let textView = responder as? UITextView {
            return textView.inputAccessoryView
        }
        if let searchBar = responder as? UISearchBar {
            return searchBar.inputAccessoryView
        }

        return nil
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
        UIApplication.shared.sendAction(
            #selector(findFirstResponder(_:)), to: nil, from: nil, for: nil
        )
        return currentResponder
    }

    @objc private func findFirstResponder(_: Any) {
        UIResponder.currentResponder = self
    }
}
