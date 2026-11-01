import XCTest

/// Synchronous client for the hardware signer host that `xtask ui-test-host` runs, see
/// rust/xtask/src/ui_test_host.rs
struct HardwareSignerControl {
    /// A signed PSBT as base64 and as the frames of an animated QR in each format
    struct SignedFrames: Decodable {
        let psbt: String
        let ur: [String]
        let bbqr: [String]

        func frames(_ format: QrFormat) -> [String] {
            switch format {
            case .ur: ur
            case .bbqr: bbqr
            }
        }
    }

    struct Descriptors: Decodable {
        let external: String
        let `internal`: String
    }

    private struct Pasteboard: Decodable {
        let text: String
    }

    /// The bridge the app's debug TAPSIGNER seam connects to
    static let tapSignerBridge = "127.0.0.1:39011"

    /// The emulated card's CVC
    static let tapSignerCvc = "123456"

    private let baseUrl = URL(string: "http://127.0.0.1:39010")!

    func requireHealthy() throws {
        do {
            _ = try controlRequest(baseUrl, "GET", "health")
        } catch {
            throw ControlRequestError(
                description: "the hardware signer host is not reachable at \(baseUrl), run this test through `just ios-ui-background`: \(error)"
            )
        }
    }

    /// Signs the PSBT with the test mnemonic, as a hardware wallet holding it would
    func sign(_ psbt: String, minParts: Int) throws -> SignedFrames {
        let body: [String: Any] = ["psbt": psbt, "mnemonic": XCUIApplication.testMnemonic, "min_parts": minParts]
        return try decode(SignedFrames.self, controlRequest(baseUrl, "POST", "sign-psbt", body: body))
    }

    /// A PSBT signed by the test mnemonic that spends a made-up output, so it matches no draft
    func unrelatedSignedPsbt(network: String, minParts: Int) throws -> SignedFrames {
        let body: [String: Any] = ["mnemonic": XCUIApplication.testMnemonic, "network": network, "min_parts": minParts]
        return try decode(SignedFrames.self, controlRequest(baseUrl, "POST", "unrelated-psbt", body: body))
    }

    /// The test mnemonic's Signet account as a hardware wallet exports it
    func watchOnlyDescriptors() throws -> Descriptors {
        let body: [String: Any] = ["mnemonic": XCUIApplication.testMnemonic]
        return try decode(Descriptors.self, controlRequest(baseUrl, "POST", "watch-only-descriptor", body: body))
    }

    /// Reads the simulator's pasteboard from the host, since reading it here shows the paste prompt
    func simulatorPasteboard() throws -> String {
        guard let udid = ProcessInfo.processInfo.environment["SIMULATOR_UDID"] else {
            throw ControlRequestError(description: "SIMULATOR_UDID is not set; this test runs only on a simulator")
        }

        let body: [String: Any] = ["udid": udid]
        return try decode(Pasteboard.self, controlRequest(baseUrl, "POST", "simulator-pasteboard", body: body)).text
    }

    /// Starts a fresh emulated TAPSIGNER, set up with the default CVC
    func startTapSigner() throws {
        _ = try controlRequest(baseUrl, "POST", "tapsigner/start")
    }

    /// Ends the next TAPSIGNER session after this many APDU exchanges, as a card pulled out of the
    /// NFC field would
    func dropNextTapSignerSession(afterExchanges count: Int) throws {
        _ = try controlRequest(baseUrl, "POST", "tapsigner/drop", body: ["after_exchanges": count])
    }

    private func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
        try JSONDecoder().decode(type, from: data)
    }
}

enum QrFormat: String {
    case ur = "UR"
    case bbqr = "BBQr"
}

/// Shows QR frames to the app's scanner through the debug QR seam
///
/// The app reads `COVE_UI_TEST_QR_SCRIPT` at launch, and while a scanner is open it takes the
/// file at that path and scans its frames in order. The simulator shares the host's file system,
/// so both processes see the same `/tmp`
struct QrScript {
    private static let environmentKey = "COVE_UI_TEST_QR_SCRIPT"

    let path = "/tmp/cove-ui-test-qr-\(UUID().uuidString).json"

    func configure(_ app: XCUIApplication) {
        app.launchEnvironment[Self.environmentKey] = path
    }

    /// Hands the frames to the open scanner, which scans them as if they were in front of the camera
    func show(_ frames: [String]) throws {
        let data = try JSONSerialization.data(withJSONObject: ["frames": frames])

        // the scanner polls for the file, so it must never see a partial write
        try data.write(to: URL(fileURLWithPath: path), options: .atomic)
    }

    func remove() {
        try? FileManager.default.removeItem(atPath: path)
    }
}
