//
//  PSBTFile.swift
//  Cove
//
//  Created by Praveen Perera on 11/24/24.
//
import Foundation
import SwiftUI
import UniformTypeIdentifiers

struct PSBTFile: Transferable {
    let data: Data
    let filename: String

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .psbt) { file in
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent(file.filename)
            try file.data.write(to: url)
            return SentTransferredFile(url)
        } importing: { url in
            Log.warn("Importing PSBT files is not supported: \(url)")
            // Importing closure - must return PSBTFile
            guard let data = try? Data(contentsOf: url.file) else {
                throw CocoaError(.fileReadUnknown)
            }

            return PSBTFile(data: data, filename: url.file.path())
        }
    }
}

extension UTType {
    static var psbt: UTType {
        UTType(exportedAs: "org.bitcoin.psbt")
    }

    static var txn: UTType {
        UTType(exportedAs: "org.bitcoin.transaction")
    }
}
