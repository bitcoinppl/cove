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
            self = try .success(await body())
        } catch {
            self = .failure(error)
        }
    }
}

public extension Result {
    func isSuccess() -> Bool {
        switch self {
        case .success: return true
        case .failure: return false
        }
    }
}
