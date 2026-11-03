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
        try withAccess { try String(contentsOf: url, encoding: .utf8) }
    }

    /// Read the raw bytes, for files that may be binary such as signed PSBTs
    func readData() throws -> Data {
        try withAccess { try Data(contentsOf: url) }
    }

    private func withAccess<T>(_ body: () throws -> T) throws -> T {
        guard url.startAccessingSecurityScopedResource() else {
            throw FileReadError(
                message: "Failed to access the file at \(url.path)"
            )
        }

        defer { url.stopAccessingSecurityScopedResource() }
        return try body()
    }
}
