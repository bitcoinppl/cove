import CoreImage
import Vision
import XCTest

/// The other phone in a KeyTeleport transfer, played by the UI test host
///
/// The app shows its receiver request only as a QR code, so a test reads it the way a sending phone
/// would, from a screenshot (`XCUIApplication.visibleQrCodes`), and reads the receiver code from the
/// screen. The host's `/keyteleport/...` routes encrypt the response with the same `keyteleport`
/// crate the app uses, and the test shows it to the app's scanner through the debug QR seam
/// (docs/testing.md)
struct KeyTeleportPeer {
    enum Payload {
        case mnemonic(String)
        case xprv(String)

        /// COLDCARD Secure Notes & Passwords records, each a dictionary of COLDCARD's JSON fields
        case notes([[String: String]])

        var json: [String: Any] {
            switch self {
            case let .mnemonic(words): ["mnemonic": words]
            case let .xprv(xprv): ["xprv": xprv]
            case let .notes(records): ["notes": records]
            }
        }
    }

    struct Response: Decodable {
        let bbqr: String
        let url: String

        /// The response as BBQr frames, more than one when the test asked for a multi-part QR
        let parts: [String]

        /// The Teleport Password as the sending phone shows it
        let password: String
    }

    /// A receiver request the host made, as a receiving phone shows it
    struct ReceiverRequest: Decodable {
        /// Names the host's session for `decode`
        let session: Int
        let bbqr: String
        let url: String

        /// The receiver code, eight digits without grouping
        let code: String
    }

    /// What the host decrypted from the app's response
    struct Received: Decodable {
        let kind: String
        let mnemonic: String?
        let xprv: String?
        let titles: [String]?
    }

    private let baseUrl = URL(string: "http://127.0.0.1:39010")!

    /// Encrypts the payload for the receiver request, as a sending phone does
    func send(_ payload: Payload, to request: String, code: String, minParts: Int = 1) throws -> Response {
        let body: [String: Any] = ["request": request, "code": code, "payload": payload.json, "min_parts": minParts]
        return try JSONDecoder().decode(Response.self, from: controlRequest(baseUrl, "POST", "keyteleport/send", body: body))
    }

    /// Starts a receive session for the app to send to, as a receiving phone does
    func startReceiving() throws -> ReceiverRequest {
        try JSONDecoder().decode(ReceiverRequest.self, from: controlRequest(baseUrl, "POST", "keyteleport/receive-request", body: [:]))
    }

    /// Decrypts the app's response to a session from `startReceiving` with the password the app showed
    func decode(_ response: String, session: Int, password: String) throws -> Received {
        let body: [String: Any] = ["session": session, "response": response, "password": password]
        return try JSONDecoder().decode(Received.self, from: controlRequest(baseUrl, "POST", "keyteleport/receive-response", body: body))
    }
}

extension XCUIApplication {
    /// Decodes every QR code a person could scan off the screen right now
    ///
    /// A concealed KeyTeleport QR code is blurred, so it doesn't decode; that it doesn't is what
    /// keeps a second phone's camera from reading it before the user reveals it
    var visibleQrCodes: [String] {
        guard let image = CIImage(image: XCUIScreen.main.screenshot().image) else { return [] }

        let detector = CIDetector(ofType: CIDetectorTypeQRCode, context: nil, options: [CIDetectorAccuracy: CIDetectorAccuracyHigh])
        return detector?.features(in: image).compactMap { ($0 as? CIQRCodeFeature)?.messageString } ?? []
    }

    /// Every line of text a person could read off the screen right now, as Vision recognizes it
    ///
    /// The accessibility tree hides a concealed value, but only the pixels show whether its blur
    /// actually makes it unreadable, so concealment checks read the screenshot instead
    var visibleText: [String] {
        guard let image = XCUIScreen.main.screenshot().image.cgImage else { return [] }

        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        try? VNImageRequestHandler(cgImage: image).perform([request])

        return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
    }

    /// The screen's text with whitespace removed, so a grouped value reads the same however Vision
    /// splits it
    var visibleTextWithoutSpaces: String {
        visibleText.joined().filter { !$0.isWhitespace }.uppercased()
    }

    /// Waits for Vision to read the text, compared without whitespace, anywhere on screen
    func waitForVisibleText(containing compact: String, timeout: TimeInterval = 5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if visibleTextWithoutSpaces.contains(compact) { return true }

            Thread.sleep(forTimeInterval: 0.3)
        }

        return false
    }

    /// Waits for a QR code to show on screen and returns what it encodes
    func waitForVisibleQrCode(timeout: TimeInterval = 10) -> String? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let code = visibleQrCodes.first { return code }

            Thread.sleep(forTimeInterval: 0.3)
        }

        return nil
    }

    /// Waits until no QR code on screen decodes
    func waitForNoVisibleQrCode(timeout: TimeInterval = 5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if visibleQrCodes.isEmpty { return true }

            Thread.sleep(forTimeInterval: 0.3)
        }

        return false
    }
}
