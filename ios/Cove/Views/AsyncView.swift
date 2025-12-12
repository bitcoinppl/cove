//
//  AsyncView.swift
//  Cove
//
//  Created by Praveen Perera on 9/15/24.
//

import SwiftUI

/// Text that shows a loading spinner when the value is nil
struct AsyncText: View {
    let text: String?
    var font: Font = .body
    var color: Color = .primary
    var spinnerScale: CGFloat = 1.0

    var body: some View {
        if let text {
            Text(text)
                .font(font)
                .foregroundColor(color)
        } else {
            ProgressView()
                .tint(color)
                .scaleEffect(spinnerScale)
        }
    }
}

struct AsyncView<Success, Content: View>: View {
    let operation: () async throws -> Success
    let content: (Success) -> Content
    let errorView: some View = Text("")

    @State private var result: Result<Success, Error>?

    var body: some View {
        Group {
            switch result {
            case .none:
                ProgressView()
                    .tint(.primary)
            case let .success(value):
                content(value)
            case .failure:
                errorView
            }
        }
        .task {
            do {
                let value = try await operation()
                result = .success(value)
            } catch {
                Log.error("Error loading async view :\(error)")
                result = .failure(error)
            }
        }
    }
}
