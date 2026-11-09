import XCTest

/// Drives the simulator's Face ID through the UI test host, which posts the notifications the
/// Simulator app's Features > Face ID menu does, see `simulator_biometrics` in
/// rust/xtask/src/ui_test_host.rs
struct SimulatorBiometrics {
    private let baseUrl = UITestHostPorts.signerControl

    /// Enrolls a face, so the app sees Face ID as available
    func enroll() throws {
        try send("enroll")
    }

    /// Removes the enrolled face; a test that enrolled one calls this in tearDown, since a face left
    /// enrolled changes which unlock and Settings screens the app shows
    func unenroll() throws {
        try send("unenroll")
    }

    /// Answers the Face ID prompt on screen with the enrolled face
    func match() throws {
        try send("match")
    }

    /// Answers the Face ID prompt on screen with a face that isn't enrolled
    func nomatch() throws {
        try send("nomatch")
    }

    private func send(_ action: String) throws {
        guard let udid = ProcessInfo.processInfo.environment["SIMULATOR_UDID"] else {
            throw ControlRequestError(description: "SIMULATOR_UDID is not set; this test runs only on a simulator")
        }

        _ = try controlRequest(baseUrl, "POST", "simulator-biometrics", body: ["udid": udid, "action": action])
    }
}
