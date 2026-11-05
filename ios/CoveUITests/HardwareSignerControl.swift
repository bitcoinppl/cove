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

    /// An account's bare public key as a hardware wallet shows it, and where an import of it
    /// should start receiving
    struct PublicKeyExport: Decodable {
        /// The SLIP-132 key, such as a vpub or upub, for a plain QR
        let key: String
        /// The key as one BBQr frame
        let bbqr: String
        let firstAddress: String
        /// The first address's path, as the Receive screen shows it
        let firstDerivationPath: String

        enum CodingKeys: String, CodingKey {
            case key
            case bbqr
            case firstAddress = "first_address"
            case firstDerivationPath = "first_derivation_path"
        }
    }

    /// The address types a bare SLIP-132 key names
    enum AddressType: String {
        case nativeSegwit = "native_segwit"
        case wrappedSegwit = "wrapped_segwit"
    }

    private struct Pasteboard: Decodable {
        let text: String
    }

    /// The bridge the app's debug TAPSIGNER seam connects to
    static var tapSignerBridge: String {
        UITestHostPorts.tapSignerBridge
    }

    /// The emulated card's CVC
    static let tapSignerCvc = "123456"

    private let baseUrl = UITestHostPorts.signerControl

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

    /// The test mnemonic's Signet account of the address type as a bare public key, with the first
    /// receive address the host derives from the mnemonic itself
    func publicKeyExport(_ addressType: AddressType) throws -> PublicKeyExport {
        let body: [String: Any] = ["mnemonic": XCUIApplication.testMnemonic, "address_type": addressType.rawValue]
        return try decode(PublicKeyExport.self, controlRequest(baseUrl, "POST", "public-key-export", body: body))
    }

    /// Reads the simulator's pasteboard from the host, since reading it here shows the paste prompt
    func simulatorPasteboard() throws -> String {
        let body: [String: Any] = try ["udid": Self.simulatorUdid()]
        return try decode(Pasteboard.self, controlRequest(baseUrl, "POST", "simulator-pasteboard", body: body)).text
    }

    /// Switches the simulator to dark or light mode, which only the host can do
    func setSimulatorAppearance(dark: Bool) throws {
        let body: [String: Any] = try ["udid": Self.simulatorUdid(), "appearance": dark ? "dark" : "light"]
        _ = try controlRequest(baseUrl, "POST", "simulator-appearance", body: body)
    }

    private static func simulatorUdid() throws -> String {
        guard let udid = ProcessInfo.processInfo.environment["SIMULATOR_UDID"] else {
            throw ControlRequestError(description: "SIMULATOR_UDID is not set; this test runs only on a simulator")
        }

        return udid
    }

    /// Starts a fresh emulated TAPSIGNER with the default CVC, set up with a key unless
    /// `needsSetup`, in which case it is a new card the app sets up
    func startTapSigner(needsSetup: Bool = false) throws {
        _ = try controlRequest(baseUrl, "POST", "tapsigner/start", body: ["needs_setup": needsSetup])
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

    /// Waits until the scanner takes the last frames shown and has had time to scan them, for
    /// checks that the scan didn't change
    func waitUntilScanned(frameCount: Int) {
        let deadline = Date().addingTimeInterval(10)
        while FileManager.default.fileExists(atPath: path), Date() < deadline {
            Thread.sleep(forTimeInterval: 0.1)
        }

        XCTAssertFalse(FileManager.default.fileExists(atPath: path), "the scanner never took the QR frames")

        // the scanner feeds the frames it took 150 ms apart
        Thread.sleep(forTimeInterval: 0.15 * Double(frameCount) + 0.5)
    }

    func remove() {
        try? FileManager.default.removeItem(atPath: path)
    }
}
