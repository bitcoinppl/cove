//
//  Logger.swift
//  Cove
//
//  Created by Praveen Perera on 7/2/24.
//

import Foundation
import OSLog

enum LogLevel: String {
    case debug, info, warn, error
}

struct Log {
    private let logger: Logger

    static let shared = Logger(subsystem: subsystem, category: "shared")
    static let viewCycle = Logger(subsystem: subsystem, category: "viewcycle")
    static let networking = Logger(subsystem: subsystem, category: "networking")

    private static let subsystem = Bundle.main.bundleIdentifier!

    init(id: String) {
        logger = Logger(subsystem: "org.bitcoinppl.cove", category: id)
    }

    // MARK: - Create specific loggers for different domains

    func debug(_ message: String) {
        #if DEBUG
            logger.debug("[swift][debug]: \(message)")
        #endif
    }

    func notice(_ message: String) {
        logger.notice("[swift][notice]: \(message)")
    }

    func info(_ message: String) {
        logger.info("[swift][info]: \(message)")
    }

    func warn(_ message: String) {
        logger.warning("[swift][warn]: \(message)")
    }

    func error(_ message: String) {
        logger.error("[swift][error]: \(message)")
    }

    // MARK: - Shared Instance for convenience

    static func debug(_ message: String) {
        #if DEBUG
            Log.shared.debug("[swift][debug]: \(message)")
        #endif
    }

    static func info(_ message: String) {
        Log.shared.info("[swift][info]: \(message)")
    }

    static func warn(_ message: String) {
        Log.shared.warning("[swift][warn]: \(message)")
    }

    static func notice(_ message: String) {
        Log.shared.notice("[swift][notice]: \(message)")
    }

    static func error(_ message: String) {
        Log.shared.error("[swift][error]: \(message)")
    }
}
