//
//  WeakReconciler.swift
//  Cove
//
//  Created by Praveen Perera on 2024-07-30.
//

import Foundation

/// Takes a weak reference to a model that conform to the AnyReconciler protocol
final class WeakReconciler<Reconciler: AnyObject & AnyReconciler, Message>: AnyReconciler, @unchecked Sendable
    where Reconciler.Message == Message
{
    private weak var reconciler: Reconciler?

    init(_ reconciler: Reconciler) {
        self.reconciler = reconciler
    }

    func reconcile(message: Message) {
        reconciler?.reconcile(message: message)
    }

    func reconcileMany(messages: [Message]) {
        reconciler?.reconcileMany(messages: messages)
    }
}

protocol AnyReconciler: AnyObject {
    associatedtype Message
    func reconcile(message: Message)
    func reconcileMany(messages: [Message])
}

protocol ReconcilingManager: AnyReconciler {
    func apply(_ message: Message)
    func logReconcile(message: Message)
    func logReconcileMany(messages: [Message])
    var canApplyReconcileMessages: Bool { get }
}

extension ReconcilingManager {
    var canApplyReconcileMessages: Bool {
        true
    }

    func logReconcile(message _: Message) {}

    func logReconcileMany(messages _: [Message]) {}

    func reconcile(message: Message) {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.canApplyReconcileMessages else { return }

            self.logReconcile(message: message)
            self.apply(message)
        }
    }

    func reconcileMany(messages: [Message]) {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.canApplyReconcileMessages else { return }

            self.logReconcileMany(messages: messages)
            messages.forEach { self.apply($0) }
        }
    }
}
