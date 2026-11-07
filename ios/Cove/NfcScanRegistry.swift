import Foundation
import os

/// Every NFC reader, writer and TAPSIGNER session with a scan open, whoever owns it
///
/// Each NFC type reports its own `isScanning` here, so a scan started from a screen's own reader
/// counts the same as one started from `AppManager`'s, and a new owner can't be left out
enum NfcScanRegistry {
    /// CoreNFC calls back on its own queues and the types set `isScanning` there, so a lock guards
    /// the set rather than the main actor
    private static let scanning = OSAllocatedUnfairLock(initialState: Set<ObjectIdentifier>())

    static var isAnyScanning: Bool {
        scanning.withLock { !$0.isEmpty }
    }

    /// Records whether the NFC object `owner` identifies has a scan open
    static func set(_ owner: ObjectIdentifier, isScanning: Bool) {
        scanning.withLock { owners in
            if isScanning {
                owners.insert(owner)
            } else {
                owners.remove(owner)
            }
        }
    }
}
