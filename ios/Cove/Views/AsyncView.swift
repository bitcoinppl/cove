//
//  AsyncView.swift
//  Cove
//
//  Created by Praveen Perera on 9/15/24.
//

import SwiftUI

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
            case .success(let value):
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
