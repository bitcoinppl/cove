//
//  Logger.swift
//  Cove
//
//  Created by Praveen Perera on 7/2/24.
//

import Foundation
import OSLog

enum LogLevel: String {
    case debug, notice, info, warn, error
}

struct Log {
    private let category: String
    private let logger: Logger

    static let shared = Logger(subsystem: subsystem, category: "shared")
    static let viewCycle = Logger(subsystem: subsystem, category: "viewcycle")
    static let networking = Logger(subsystem: subsystem, category: "networking")

    private static let subsystem = Bundle.main.bundleIdentifier!

    init(id: String) {
        category = id
        logger = Logger(subsystem: "org.bitcoinppl.cove", category: id)
    }

    // MARK: - Create specific loggers for different domains

    func debug(_ message: String) {
        #if DEBUG
            Self.record(level: .debug, category: category, message: message)
            logger.debug("\(Self.osLogMessage(level: .debug, message: message))")
        #endif
    }

    func notice(_ message: String) {
        Self.record(level: .notice, category: category, message: message)
        logger.notice("\(Self.osLogMessage(level: .notice, message: message))")
    }

    func info(_ message: String) {
        Self.record(level: .info, category: category, message: message)
        logger.info("\(Self.osLogMessage(level: .info, message: message))")
    }

    func warn(_ message: String) {
        Self.record(level: .warn, category: category, message: message)
        logger.warning("\(Self.osLogMessage(level: .warn, message: message))")
    }

    func error(_ message: String) {
        Self.record(level: .error, category: category, message: message)
        logger.error("\(Self.osLogMessage(level: .error, message: message))")
    }

    // MARK: - Shared Instance for convenience

    static func debug(_ message: String) {
        #if DEBUG
            record(level: .debug, category: "shared", message: message)
            Log.shared.debug("\(osLogMessage(level: .debug, message: message))")
        #endif
    }

    static func info(_ message: String) {
        record(level: .info, category: "shared", message: message)
        Log.shared.info("\(osLogMessage(level: .info, message: message))")
    }

    static func warn(_ message: String) {
        record(level: .warn, category: "shared", message: message)
        Log.shared.warning("\(osLogMessage(level: .warn, message: message))")
    }

    static func notice(_ message: String) {
        record(level: .notice, category: "shared", message: message)
        Log.shared.notice("\(osLogMessage(level: .notice, message: message))")
    }

    static func error(_ message: String) {
        record(level: .error, category: "shared", message: message)
        Log.shared.error("\(osLogMessage(level: .error, message: message))")
    }

    private static func record(level: LogLevel, category: String, message: String) {
        SwiftLogStore.shared.record(level: level, category: category, message: message)
    }

    private static func osLogMessage(level: LogLevel, message: String) -> String {
        "[swift][\(level.rawValue)]: \(message)"
    }
}
