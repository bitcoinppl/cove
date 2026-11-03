//
//  UITestQrScript.swift
//  Cove
//
//  Debug-only stand-in for the camera in UI tests
//

#if DEBUG
    import Foundation

    /// QR frames a UI test feeds the scanner instead of the camera
    ///
    /// When `COVE_UI_TEST_QR_SCRIPT` names a file, every open scanner polls it.
    /// The test writes `{"frames": ["…", …]}` there, atomically; the scanner
    /// takes the file and feeds the frames one at a time through the path a
    /// camera detection takes, so progress, haptics, errors and completion all
    /// behave as they do with a real scan. Writing the file again feeds more
    /// frames to the same scan
    ///
    /// The variable is read from the launch environment, which XCUITest sets
    /// with `XCUIApplication.launchEnvironment`. The simulator shares this
    /// machine's file system, so a path under `/tmp` works for both processes
    enum UITestQrScript {
        static let environmentKey = "COVE_UI_TEST_QR_SCRIPT"
        static let frameInterval = Duration.milliseconds(150)
        static let pollInterval = Duration.milliseconds(200)

        private struct Script: Decodable {
            let frames: [String]
        }

        static var path: String? {
            ProcessInfo.processInfo.environment[environmentKey]
        }

        /// Takes the scripted frames, or returns nil until the test writes some
        static func takeFrames(at path: String) -> [String]? {
            guard let data = FileManager.default.contents(atPath: path) else { return nil }
            try? FileManager.default.removeItem(atPath: path)

            do {
                return try JSONDecoder().decode(Script.self, from: data).frames
            } catch {
                Log.error("UI test QR script at \(path) is not valid: \(error)")
                return nil
            }
        }
    }
#endif
