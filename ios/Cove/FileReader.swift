//
//  FileReader.swift
//  Cove
//
//  Created by Praveen Perera on 11/26/24.
//

import Foundation

public struct FileReadError: Error {
    let message: String
}

public struct FileReader {
    var url: URL

    init(for url: URL) {
        self.url = url
    }

    func read() throws -> String {
        guard url.startAccessingSecurityScopedResource() else {
            throw FileReadError(
                message: "Failed to access the file at \(url.path)"
            )
        }

        defer { url.stopAccessingSecurityScopedResource() }
        return try String(contentsOf: url, encoding: .utf8)
    }
}
