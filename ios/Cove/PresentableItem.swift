//
//  PresentableItem.swift
//  Cove
//
//  Created by Praveen Perera on 10/20/24.
//
import Foundation

struct PresentableItem<T>: Identifiable {
    let id: UUID
    let item: T

    init(_ item: T) {
        self.id = UUID()
        self.item = item
    }
}
