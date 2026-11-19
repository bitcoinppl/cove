//
//  UITestCameraPrompt.swift
//  Cove
//
//  Debug-only camera permission request for UI tests on the simulator
//

#if DEBUG
    import Foundation

    /// Makes the simulator's scanner ask for camera access, as the scanner on a phone does
    ///
    /// The simulator's scanner skips the camera, so it never asks for access, and neither the
    /// system prompt nor the answer it sends back ever reach the app. When
    /// `COVE_UI_TEST_CAMERA_PROMPT` is set in the launch environment, the simulator's scanner
    /// makes the phone's request from `viewDidLoad`, so the prompt shows while the scanner's sheet
    /// slides in and the answer comes back through the same code
    enum UITestCameraPrompt {
        static let environmentKey = "COVE_UI_TEST_CAMERA_PROMPT"

        static var isEnabled: Bool {
            ProcessInfo.processInfo.environment[environmentKey] != nil
        }
    }
#endif
