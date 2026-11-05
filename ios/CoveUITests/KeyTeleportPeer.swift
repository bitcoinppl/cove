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
        /// The response as BBQr frames, more than one when the test asked for a multi-part QR or
        /// the payload is too big for one
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

extension XCUIElement {
    /// Decodes every QR code a person could scan off the element's part of the screen right now
    ///
    /// A concealed KeyTeleport QR code is blurred, so it doesn't decode; that it doesn't is what
    /// keeps a second phone's camera from reading it before the user reveals it
    var visibleQrCodes: [String] {
        guard let image = CIImage(image: screenshot().image) else {
            XCTFail("the screenshot of \(self) has no image to decode")
            return []
        }

        let detector = CIDetector(ofType: CIDetectorTypeQRCode, context: nil, options: [CIDetectorAccuracy: CIDetectorAccuracyHigh])
        return detector?.features(in: image).compactMap { ($0 as? CIQRCodeFeature)?.messageString } ?? []
    }

    /// Every line of text a person could read off the element's part of the screen right now, as
    /// Vision recognizes it
    ///
    /// The accessibility tree hides a concealed value, but only the pixels show whether its blur
    /// actually makes it unreadable, so concealment checks read the screenshot instead. A screenshot
    /// Vision can't read fails the test, so no check passes because nothing was read
    var visibleText: [String] {
        guard let image = screenshot().image.cgImage else {
            XCTFail("the screenshot of \(self) has no image to read")
            return []
        }

        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false

        do {
            try VNImageRequestHandler(cgImage: image).perform([request])
        } catch {
            XCTFail("Vision could not read the screenshot of \(self): \(error)")
            return []
        }

        return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
    }

    /// The text with whitespace removed, so a grouped value reads the same however Vision splits it
    var visibleTextWithoutSpaces: String {
        visibleText.joined().withoutSpaces
    }

    /// Waits for Vision to read the text, compared without whitespace and case
    func waitForVisibleText(containing text: String, timeout: TimeInterval = 5) -> Bool {
        let compact = text.withoutSpaces
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if visibleTextWithoutSpaces.contains(compact) { return true }

            Thread.sleep(forTimeInterval: 0.3)
        }

        return false
    }

    /// Waits for a QR code to show and returns what it encodes
    func waitForVisibleQrCode(timeout: TimeInterval = 10) -> String? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let code = visibleQrCodes.first { return code }

            Thread.sleep(forTimeInterval: 0.3)
        }

        return nil
    }

    /// Waits until no QR code decodes
    func waitForNoVisibleQrCode(timeout: TimeInterval = 5) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if visibleQrCodes.isEmpty { return true }

            Thread.sleep(forTimeInterval: 0.3)
        }

        return false
    }

    /// Fails if the element, a reveal hint drawn over a concealed value, shows any of that value:
    /// a QR code that decodes, or a line Vision reads that isn't the hint and holds two or more
    /// letters or digits
    ///
    /// Codes and passwords show in pairs, so two characters is the least of one that can leak
    /// past the hint even where it covers the middle of the value
    func assertConceals(hint: String, _ name: String) {
        XCTAssertTrue(visibleQrCodes.isEmpty, "\(name): a QR code decodes while it is hidden")

        let leaked = visibleText.filter { line in
            !line.readsAs(hint) && line.filter { $0.isLetter || $0.isNumber }.count >= 2
        }

        XCTAssertTrue(leaked.isEmpty, "\(name): Vision read \(leaked) while it was hidden")
    }
}

private extension String {
    var withoutSpaces: String {
        filter { !$0.isWhitespace }.uppercased()
    }

    /// Whether Vision's reading of the line is the hint, allowing a few misread or extra
    /// characters such as the eye icon read as a letter
    func readsAs(_ hint: String) -> Bool {
        let read = Array(filter { $0.isLetter || $0.isNumber }.uppercased())
        let expected = Array(hint.filter { $0.isLetter || $0.isNumber }.uppercased())
        return Self.editDistance(read, expected) <= 3
    }

    static func editDistance(_ lhs: [Character], _ rhs: [Character]) -> Int {
        var previous = Array(0 ... rhs.count)
        for (i, left) in lhs.enumerated() {
            var current = [i + 1]
            for (j, right) in rhs.enumerated() {
                current.append(Swift.min(previous[j + 1] + 1, current[j] + 1, previous[j] + (left == right ? 0 : 1)))
            }

            previous = current
        }

        return previous[rhs.count]
    }
}

extension ScreenReachability {
    /// Reveals a KeyTeleport QR code and decodes it from a screenshot, then reveals the code or
    /// password beside it, as the other phone's user would; returns both, or nil if the QR code
    /// never decoded
    ///
    /// On the way it checks the pixels under each hint: neither shows any of itself until the user
    /// reveals it, and revealing either one, in both orders, conceals the other. Vision must read
    /// the code once shown, which proves it could have read it while hidden. It ends with the QR
    /// code showing
    func readRevealPair(qrHint: String, codeHint: String, code: XCUIElement, _ name: String) -> (qr: String, code: String)? {
        XCTAssertFalse(code.exists, "\(name): the code must stay hidden until the user reveals it")
        cover(qrHint, name).assertConceals(hint: qrHint, "\(name): the QR code before either is revealed")
        cover(codeHint, name).assertConceals(hint: codeHint, "\(name): the code before either is revealed")

        cover(qrHint, name).tap()
        guard let qr = app.waitForVisibleQrCode() else {
            XCTFail("\(name): the revealed QR code never decoded")
            return nil
        }

        XCTAssertFalse(code.exists, "\(name): the code must stay hidden while the QR code shows")
        cover(codeHint, name).assertConceals(hint: codeHint, "\(name): the code while the QR code shows")

        cover(codeHint, name).tap()
        XCTAssertTrue(code.waitForExistence(timeout: 5), "\(name): the code never showed")
        XCTAssertTrue(app.waitForNoVisibleQrCode(), "\(name): showing the code must make the QR code unreadable again")
        cover(qrHint, name).assertConceals(hint: qrHint, "\(name): the QR code while the code shows")
        let shownCode = code.label
        XCTAssertTrue(code.waitForVisibleText(containing: shownCode), "\(name): Vision never read the code once shown")

        // and back, so revealing the QR code again conceals the code it replaces
        cover(qrHint, name).tap()
        XCTAssertNotNil(app.waitForVisibleQrCode(), "\(name): the QR code never showed again")
        XCTAssertTrue(code.waitForNonExistence(timeout: 5), "\(name): showing the QR code again must hide the code")
        cover(codeHint, name).assertConceals(hint: codeHint, "\(name): the code once the QR code shows again")

        return (qr, shownCode)
    }

    /// The hint over a concealed value, scrolled on screen so its screenshot shows what it covers
    private func cover(_ hint: String, _ name: String) -> XCUIElement {
        assertReachable(app.buttons.matching(NSPredicate(format: "label == %@", hint)), "\(name): '\(hint)'", needsHit: true)
    }
}
