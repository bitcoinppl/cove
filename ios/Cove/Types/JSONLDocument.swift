import Foundation
import SwiftUI
import UniformTypeIdentifiers

struct JSONLDocument: FileDocument {
    static var readableContentTypes = [UTType.jsonl, UTType.json, UTType.plainText]
    var text: String

    init(text: String) {
        self.text = text
    }

    init(configuration: ReadConfiguration) throws {
        guard let data = configuration.file.regularFileContents,
              let string = String(data: data, encoding: .utf8)
        else {
            throw CocoaError(.fileReadCorruptFile)
        }
        text = string
    }

    func fileWrapper(configuration _: WriteConfiguration) throws -> FileWrapper {
        guard let data = text.data(using: .utf8)
        else {
            throw CocoaError(.fileWriteInapplicableStringEncoding)
        }
        return FileWrapper(regularFileWithContents: data)
    }
}

extension UTType {
    static var jsonl: UTType {
        UTType(exportedAs: "public.jsonl")
    }
}
