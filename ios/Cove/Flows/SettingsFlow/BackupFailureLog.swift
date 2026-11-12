//
//  BackupFailureLog.swift
//  Cove
//

import Foundation

/// Logs a backup failure that did not come from the backup manager
///
/// The Rust backup manager logs every `BackupError` it returns, so only other failures, such as
/// a file read or binding error, would otherwise reach the screen without a log
func logUnexpectedBackupFailure(_ logger: Log, _ operation: String, _ error: Error) {
    if error is BackupError { return }

    logger.warn("backup \(operation) failed: \(error.logSummary)")
}
