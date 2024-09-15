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

    @State private var result: Result<Success, Error>?

    var body: some View {
        Group {
            switch result {
            case .none:
                ProgressView()
            case .success(let value):
                content(value)
            case .failure(let error):
                Text("Error: \(error.localizedDescription)")
            }
        }
        .task {
            do {
                let value = try await operation()
                result = .success(value)
            } catch {
                result = .failure(error)
            }
        }
    }
}
