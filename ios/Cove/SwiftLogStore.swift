import CoveCore
import Foundation

final class SwiftLogStore {
    static let shared = SwiftLogStore(
        logsDirectory: URL(fileURLWithPath: rootDataDirPath(), isDirectory: true)
            .appendingPathComponent("logs", isDirectory: true)
    )

    private static let logFileBytes = 256 * 1024
    private static let maxTotalFileBytes = 2 * 1024 * 1024
    private static let archiveFileCount = (maxTotalFileBytes / logFileBytes) - 1
    private static let currentLogFile = "cove-swift.log"
    private static let fallbackText = "no Swift logs captured\n"

    private let logsDirectory: URL
    private let fileManager: FileManager
    private let queue: DispatchQueue
    private var currentSize: Int?
    private var lastWriteError: String?

    init(
        logsDirectory: URL,
        fileManager: FileManager = .default,
        queueLabel: String = "org.bitcoinppl.cove.swift-log-store"
    ) {
        self.logsDirectory = logsDirectory
        self.fileManager = fileManager
        queue = DispatchQueue(label: queueLabel)
    }

    func record(level: LogLevel, category: String, message: String) {
        let line = Self.line(level: level, category: category, message: message)

        queue.async {
            do {
                try self.writeEntry(line)
            } catch {
                self.lastWriteError = error.localizedDescription
            }
        }
    }

    func snapshot() -> String {
        queue.sync {
            var text = ""
            if let lastWriteError {
                text.append("failed to write Swift diagnostics log file: \(lastWriteError)\n")
            }

            text += logFileURLs().reduce(into: "") { snapshot, url in
                guard fileManager.fileExists(atPath: url.path) else { return }

                do {
                    try snapshot.append(String(contentsOf: url, encoding: .utf8))
                } catch {
                    snapshot.append("failed to read Swift diagnostics log file: \(error)\n")
                }
            }

            return text.isEmpty ? Self.fallbackText : text
        }
    }

    func clear() throws {
        try queue.sync {
            currentSize = nil

            do {
                for url in allLogFileURLs() {
                    try removeFileIfExists(url)
                }

                lastWriteError = nil
                try writeEntry("swift diagnostics logs cleared at \(Self.timestamp())\n")
            } catch {
                lastWriteError = error.localizedDescription

                throw error
            }
        }
    }

    private static func line(level: LogLevel, category: String, message: String) -> String {
        "\(timestamp()) \(level.rawValue.uppercased()) \(category): \(sanitized(message))\n"
    }

    private static func timestamp() -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]

        return formatter.string(from: Date())
    }

    private static func sanitized(_ message: String) -> String {
        message
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
    }

    private func writeEntry(_ entry: String) throws {
        try fileManager.createDirectory(
            at: logsDirectory,
            withIntermediateDirectories: true
        )

        let entry = Self.entryForFile(entry)
        let data = Data(entry.utf8)

        if currentSize == nil {
            currentSize = fileSize(currentLogURL())
        }

        if let size = currentSize, size > 0, size + data.count > Self.logFileBytes {
            try rotate()
        }

        if !fileManager.fileExists(atPath: currentLogURL().path) {
            fileManager.createFile(
                atPath: currentLogURL().path,
                contents: nil,
                attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
            )
            currentSize = 0
        }

        let handle = try FileHandle(forWritingTo: currentLogURL())
        defer { try? handle.close() }

        try handle.seekToEnd()
        try handle.write(contentsOf: data)
        currentSize = (currentSize ?? 0) + data.count
    }

    private static func entryForFile(_ entry: String) -> String {
        guard entry.utf8.count > logFileBytes else { return entry }

        return lastBytesAtTokenBoundary(entry, maxBytes: logFileBytes)
    }

    private static func lastBytesAtTokenBoundary(_ value: String, maxBytes: Int) -> String {
        var bytes = Array(value.utf8.suffix(maxBytes))

        while !bytes.isEmpty, String(bytes: bytes, encoding: .utf8) == nil {
            bytes.removeFirst()
        }

        guard var text = String(bytes: bytes, encoding: .utf8) else { return "" }

        while let first = text.unicodeScalars.first, first.isASCII, CharacterSet.alphanumerics.contains(first) {
            text.removeFirst()
        }

        return text
    }

    private func rotate() throws {
        try removeFileIfExists(archivedLogURL(Self.archiveFileCount))

        for index in stride(from: Self.archiveFileCount - 1, through: 1, by: -1) {
            try renameIfExists(from: archivedLogURL(index), to: archivedLogURL(index + 1))
        }

        try renameIfExists(from: currentLogURL(), to: archivedLogURL(1))
        currentSize = 0
    }

    private func renameIfExists(from source: URL, to destination: URL) throws {
        guard fileManager.fileExists(atPath: source.path) else { return }

        try fileManager.moveItem(at: source, to: destination)
    }

    private func removeFileIfExists(_ url: URL) throws {
        guard fileManager.fileExists(atPath: url.path) else { return }

        try fileManager.removeItem(at: url)
    }

    private func fileSize(_ url: URL) -> Int {
        let attributes = try? fileManager.attributesOfItem(atPath: url.path)

        return (attributes?[.size] as? NSNumber)?.intValue ?? 0
    }

    private func logFileURLs() -> [URL] {
        (1 ... Self.archiveFileCount)
            .reversed()
            .map(archivedLogURL)
            + [currentLogURL()]
    }

    private func allLogFileURLs() -> [URL] {
        [currentLogURL()] + (1 ... Self.archiveFileCount).map(archivedLogURL)
    }

    private func currentLogURL() -> URL {
        logsDirectory.appendingPathComponent(Self.currentLogFile)
    }

    private func archivedLogURL(_ index: Int) -> URL {
        logsDirectory.appendingPathComponent("cove-swift.\(index).log")
    }
}
