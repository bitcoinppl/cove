//
//  ExportTypes.swift
//  Cove
//
//  Created by Praveen Perera on 11/24/25.
//

import Foundation
import SwiftUI
import UniformTypeIdentifiers

// MARK: - Labels Export

struct LabelsExport: Transferable {
    let content: String
    let filename: String

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .jsonl) { export in
            // ensure filename has .jsonl extension
            let filename = export.filename.hasSuffix(".jsonl") ? export.filename : "\(export.filename).jsonl"
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent(filename)
            guard let data = export.content.data(using: .utf8) else {
                throw ExportError.encodingFailed
            }
            try data.write(to: url)
            return SentTransferredFile(url)
        } importing: { url in
            Log.warn("Importing label files is not supported: \(url)")
            guard let data = try? Data(contentsOf: url.file),
                  let content = String(data: data, encoding: .utf8)
            else {
                throw CocoaError(.fileReadUnknown)
            }
            return LabelsExport(content: content, filename: url.file.lastPathComponent)
        }
    }
}

// MARK: - Backup Export

struct BackupExport: Transferable {
    let content: String
    let filename: String

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .plainText) { export in
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent(export.filename)
            guard let data = export.content.data(using: .utf8) else {
                throw ExportError.encodingFailed
            }
            try data.write(to: url)
            return SentTransferredFile(url)
        } importing: { url in
            Log.warn("Importing backup files is not supported: \(url)")
            guard let data = try? Data(contentsOf: url.file),
                  let content = String(data: data, encoding: .utf8)
            else {
                throw CocoaError(.fileReadUnknown)
            }
            return BackupExport(content: content, filename: url.file.lastPathComponent)
        }
    }
}

// MARK: - Export Error

enum ExportError: Error {
    case encodingFailed
    case exportFailed
}
