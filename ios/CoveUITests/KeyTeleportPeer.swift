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
        recognizedText.compactMap { $0.topCandidates(1).first?.string }
    }

    private var recognizedText: [VNRecognizedTextObservation] {
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

        return (request.results ?? []).inReadingOrder
    }

    /// The text with whitespace removed, so a grouped value reads the same however Vision splits it
    var visibleTextWithoutSpaces: String {
        visibleText.joined().withoutSpaces
    }

    /// Waits for Vision to read the text, compared without whitespace and case, allowing up to
    /// `misreads` characters misread, missed or added
    func waitForVisibleText(containing text: String, misreads: Int = 0, timeout: TimeInterval = 5) -> Bool {
        let compact = Array(text.withoutSpaces)
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if Array(visibleTextWithoutSpaces).contains(compact, allowingEdits: misreads) { return true }

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
    /// a QR code that decodes, or anything Vision reads beyond the hint
    ///
    /// Vision must read the hint itself, so a screenshot it reads nothing from fails rather than
    /// passing as concealed. Its lines join in reading order, since the hint wraps at large text
    /// sizes
    func assertConceals(hint: String, _ name: String) {
        XCTAssertTrue(visibleQrCodes.isEmpty, "\(name): a QR code decodes while it is hidden")

        let lines = visibleText
        XCTAssertTrue(lines.joined(separator: " ").readsAs(hint), "\(name): Vision read \(lines) while it was hidden, not just '\(hint)'")
    }
}

private extension String {
    var withoutSpaces: String {
        filter { !$0.isWhitespace }.uppercased().latinized
    }

    /// Latin capitals in place of the Cyrillic and Greek ones drawn the same, which Vision
    /// sometimes reads a Latin letter as, such as a Cyrillic 'С' in 'Copy'
    var latinized: String {
        String(map { Self.lookalikes[$0] ?? $0 })
    }

    private static let lookalikes: [Character: Character] = [
        "А": "A", "В": "B", "Е": "E", "І": "I", "Ј": "J", "К": "K", "М": "M", "Н": "H", "О": "O", "Р": "P", "С": "C", "Ѕ": "S",
        "Т": "T", "У": "Y", "Х": "X",
        "Α": "A", "Β": "B", "Ε": "E", "Ζ": "Z", "Η": "H", "Ι": "I", "Κ": "K", "Μ": "M", "Ν": "N", "Ο": "O", "Ρ": "P", "Τ": "T", "Υ": "Y", "Χ": "X",
    ]

    /// Whether Vision's reading is the hint and nothing beyond it
    ///
    /// Vision may misread or miss up to two of the hint's letters and digits, and read the eye
    /// icon before them as one more, but read no more than the hint holds. Text alone can't tell a
    /// leaked character from a misread one, so codes and passwords, which show in pairs, can leak a
    /// pair past this only when Vision also misses one of the hint's own characters
    func readsAs(_ hint: String) -> Bool {
        let read = Array(filter { $0.isLetter || $0.isNumber }.uppercased().latinized)
        let expected = Array(hint.filter { $0.isLetter || $0.isNumber }.uppercased().latinized)
        return [read, Array(read.dropFirst())].contains { reading in
            reading.count <= expected.count && Self.editDistance(reading, expected) <= 2
        }
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

private extension [Character] {
    /// Whether some stretch of these characters reads as the needle within `allowingEdits`
    /// characters misread, missed or added
    func contains(_ needle: [Character], allowingEdits edits: Int) -> Bool {
        // edit distance from the needle to the best stretch ending at each character, where a
        // stretch may start anywhere, so the first row is all zeros
        var previous = [Int](repeating: 0, count: count + 1)
        for (i, wanted) in needle.enumerated() {
            var current = [i + 1]
            for (j, read) in enumerated() {
                current.append(Swift.min(previous[j + 1] + 1, current[j] + 1, previous[j] + (wanted == read ? 0 : 1)))
            }

            previous = current
        }

        return (previous.min() ?? needle.count) <= edits
    }
}

private extension [VNRecognizedTextObservation] {
    /// Rows top to bottom, each left to right, so a wrapped hint or label joins in the order it
    /// reads; Vision doesn't promise an order
    var inReadingOrder: [VNRecognizedTextObservation] {
        var rows: [[VNRecognizedTextObservation]] = []

        // boxes have their origin at the bottom left, so the top row has the greatest y
        for observation in sorted(by: { $0.boundingBox.midY > $1.boundingBox.midY }) {
            let box = observation.boundingBox

            // a box overlapping the row's last box by half the shorter one's height continues that
            // row; wrapped lines don't overlap that much
            if let last = rows.last?.last?.boundingBox,
               Swift.min(last.maxY, box.maxY) - Swift.max(last.minY, box.minY) > Swift.min(last.height, box.height) / 2
            {
                rows[rows.count - 1].append(observation)
            } else {
                rows.append([observation])
            }
        }

        return rows.flatMap { row in row.sorted { $0.boundingBox.minX < $1.boundingBox.minX } }
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
