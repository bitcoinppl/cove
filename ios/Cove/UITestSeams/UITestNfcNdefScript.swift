//
//  UITestNfcNdefScript.swift
//  Cove
//
//  Debug-only stand-in for NFC tags in UI tests
//

#if DEBUG
    import Foundation

    /// NDEF messages a UI test holds to the NFC reader instead of a tag
    ///
    /// When `COVE_UI_TEST_NFC_NDEF_SCRIPT` names a file, an NFC scan skips the NFC session and
    /// polls that file instead. The test writes `{"ndef": "<hex>"}` there, atomically, with the raw
    /// bytes of an NDEF message; the scan takes the file and reads the message through the NDEF
    /// parsing and record processing a real tag read uses. The TAPSIGNER bridge takes precedence
    /// when both are set
    ///
    /// The variable is read from the launch environment, which XCUITest sets with
    /// `XCUIApplication.launchEnvironment`. The simulator shares this machine's file system, so a
    /// path under `/tmp` works for both processes
    enum UITestNfcNdefScript {
        static let environmentKey = "COVE_UI_TEST_NFC_NDEF_SCRIPT"
        static let pollInterval = Duration.milliseconds(200)

        private struct Script: Decodable {
            let ndef: String
        }

        static var path: String? {
            ProcessInfo.processInfo.environment[environmentKey]
        }

        /// Waits until the test writes an NDEF message and takes its bytes, or returns nil if the
        /// scan is cancelled first
        static func nextMessage(at path: String) async -> Data? {
            while !Task.isCancelled {
                if let message = takeMessage(at: path) {
                    return message
                }

                try? await Task.sleep(for: pollInterval)
            }

            return nil
        }

        private static func takeMessage(at path: String) -> Data? {
            guard let data = FileManager.default.contents(atPath: path) else { return nil }
            try? FileManager.default.removeItem(atPath: path)

            do {
                let hex = try JSONDecoder().decode(Script.self, from: data).ndef
                guard let bytes = bytes(fromHex: hex) else {
                    Log.error("UI test NDEF script at \(path) is not hex")
                    return nil
                }

                return bytes
            } catch {
                Log.error("UI test NDEF script at \(path) is not valid: \(error)")
                return nil
            }
        }

        private static func bytes(fromHex hex: String) -> Data? {
            let digits = Array(hex.utf8)
            guard digits.count.isMultiple(of: 2) else { return nil }

            var bytes = Data(capacity: digits.count / 2)
            for index in stride(from: 0, to: digits.count, by: 2) {
                guard let byte = UInt8(String(decoding: digits[index ..< index + 2], as: UTF8.self), radix: 16) else {
                    return nil
                }

                bytes.append(byte)
            }

            return bytes
        }
    }
#endif
