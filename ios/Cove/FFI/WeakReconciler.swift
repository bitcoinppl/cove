//
//  WeakReconciller.swift
//  Cove
//
//  Created by Praveen Perera on 2024-07-30.
//

import Foundation

// Takes a weak reference to a model that conform to the AnyReconciler protocol
class WeakReconciler<Reconciler: AnyObject, Message>: AnyReconciler where Reconciler: AnyReconciler, Reconciler.Message == Message {
    weak var reconciler: Reconciler?

    init(_ reconciler: Reconciler) {
        self.reconciler = reconciler
    }

    func reconcile(message: Message) {
        reconciler?.reconcile(message: message)
    }
}

protocol AnyReconciler {
    associatedtype Message
    func reconcile(message: Message)
}
