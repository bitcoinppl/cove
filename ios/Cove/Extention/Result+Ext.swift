//
//  Result+Ext.swift
//  Cove
//
//  Created by Praveen Perera on 7/22/24.
//

import Foundation

public extension Result where Failure == Swift.Error {
    init(catching body: () async throws -> Success) async {
        do {
            self = try await .success(body())
        } catch {
            self = .failure(error)
        }
    }
}

public extension Result {
    func isSuccess() -> Bool {
        switch self {
        case .success: true
        case .failure: false
        }
    }
}
