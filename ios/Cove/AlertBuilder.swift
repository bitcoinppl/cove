//
//  AlertBuilder.swift
//  Cove
//
//  Created by Praveen Perera on 11/25/24.
//
import SwiftUI

struct AnyAlertBuilder: AlertBuilderProtocol {
    let title: String
    let message: AnyView
    let actions: AnyView

    init(_ alert: some AlertBuilderProtocol) {
        title = alert.title
        message = AnyView(alert.message)
        actions = AnyView(alert.actions)
    }
}

protocol AlertBuilderProtocol {
    associatedtype Message: View
    associatedtype Actions: View

    var title: String { get }
    var message: Message { get }
    var actions: Actions { get }
}

struct AlertBuilder<Actions: View, Message: View>: AlertBuilderProtocol {
    let title: String
    let message: Message
    let actions: Actions

    init(
        title: String,
        @ViewBuilder message: () -> Message,
        @ViewBuilder actions: () -> Actions
    ) {
        self.title = title
        self.message = message()
        self.actions = actions()
    }

    init(
        title: String,
        message: String,
        @ViewBuilder actions: () -> Actions
    ) where Message == Text {
        self.title = title
        self.message = Text(message)
        self.actions = actions()
    }

    func eraseToAny() -> AnyAlertBuilder {
        AnyAlertBuilder(self)
    }
}
