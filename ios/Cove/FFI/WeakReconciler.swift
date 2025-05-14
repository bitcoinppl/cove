//
//  WeakReconciler.swift
//  Cove
//
//  Created by Praveen Perera on 2024-07-30.
//

import Foundation

// Takes a weak reference to a model that conform to the AnyReconciler protocol
final class WeakReconciler<Reconciler: AnyObject, Message>: AnyReconciler, @unchecked Sendable
    where Reconciler: AnyReconciler, Reconciler.Message == Message
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
